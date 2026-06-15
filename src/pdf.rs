//! PDF writer — pure Rust, no external PDF library.
//!
//! Emits a PDF/1.7 file containing one page per slide. Uses the 14 standard
//! PDF fonts (Helvetica family + Courier family) so no font embedding is
//! required. Content streams are compressed with FlateDecode (via flate2,
//! already a transitive dep of zip). JPEG images embed directly via
//! /DCTDecode; PNG images are decoded into raw pixel data and re-encoded
//! with /FlateDecode.

use crate::image::{self, ImageMeta};
use crate::ir::*;
use crate::layout::{Layout, LayoutKind};
use crate::syntax::{self, Token};
use crate::theme::Theme;
use anyhow::{Context, Result};
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

// EMU per point: 1 point = 12700 EMU.
const EMU_PER_PT: f32 = 12700.0;

// ---------------------------------------------------------------------------
// Public entry
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotesPageSize {
    /// Use the deck's own page size/aspect for presenter notes.
    Slide,
    /// Use A4 portrait pages for print-oriented notes.
    A4,
}

impl Default for NotesPageSize {
    fn default() -> Self {
        NotesPageSize::Slide
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotesLayout {
    /// Choose side-by-side for wide pages and below for portrait/tall pages.
    Auto,
    /// Place the slide thumbnail on the left and notes on the right.
    SideBySide,
    /// Place the slide thumbnail above the notes.
    Below,
}

impl Default for NotesLayout {
    fn default() -> Self {
        NotesLayout::Auto
    }
}

pub fn write(
    slides: &[Slide],
    theme: &Theme,
    layout: &Layout,
    deck_title: &str,
    author: &str,
    base_dir: &Path,
    logo: Option<&Path>,
    handout: Option<u32>,
    transition: Option<&str>,
    transition_dur: f32,
    direction: Option<&str>,
    with_notes: bool,
    notes_page_size: NotesPageSize,
    notes_layout: NotesLayout,
    cjk_font: Option<&Path>,
) -> Result<Vec<u8>> {
    let mut font_options = crate::font::PdfFontOptions::default();
    if let Some(path) = cjk_font {
        font_options.fallback_fonts.push(path);
    }
    write_with_font_options(
        slides,
        theme,
        layout,
        deck_title,
        author,
        base_dir,
        logo,
        handout,
        transition,
        transition_dur,
        direction,
        with_notes,
        notes_page_size,
        notes_layout,
        font_options,
    )
}

pub fn write_with_font_options(
    slides: &[Slide],
    theme: &Theme,
    layout: &Layout,
    deck_title: &str,
    author: &str,
    base_dir: &Path,
    logo: Option<&Path>,
    handout: Option<u32>,
    transition: Option<&str>,
    transition_dur: f32,
    direction: Option<&str>,
    with_notes: bool,
    notes_page_size: NotesPageSize,
    notes_layout: NotesLayout,
    font_options: crate::font::PdfFontOptions<'_>,
) -> Result<Vec<u8>> {
    // RTL note kept brief — DejaVu Sans (embedded for PDF) covers Arabic
    // and Hebrew glyphs, so the only missing piece for full RTL PDF is
    // joining/shaping behaviour from the viewer's text layout engine.
    let _ = direction;
    let mut metas: Vec<ImageMeta> = Vec::new();
    let mut by_src: HashMap<String, usize> = HashMap::new();
    for (idx, slide) in slides.iter().enumerate() {
        let slide_num = idx + 1;
        collect_block_images(&slide.blocks, base_dir, &mut metas, &mut by_src)
            .with_context(|| format!("on slide {} ({:?})", slide_num, slide.title))?;
        if let Some(bg) = &slide.bg_image {
            if !by_src.contains_key(bg) {
                let path = base_dir.join(bg);
                let meta = image::load(&path).with_context(|| {
                    format!("loading background image {} (slide {})", bg, slide_num)
                })?;
                by_src.insert(bg.clone(), metas.len());
                metas.push(meta);
            }
        }
    }
    let logo_key: Option<String> = if let Some(p) = logo {
        let key = format!("__logo:{}", p.display());
        if !by_src.contains_key(&key) {
            let meta =
                image::load(p).with_context(|| format!("loading logo image {}", p.display()))?;
            by_src.insert(key.clone(), metas.len());
            metas.push(meta);
        }
        Some(key)
    } else {
        None
    };

    let mut decoded_images: Vec<DecodedImage> = Vec::new();
    for m in &metas {
        decoded_images.push(decode_image(m)?);
    }

    let mut pdf = PdfWriter::new(deck_title, author, theme.slide_w, theme.slide_h);

    // Reserve object ids.
    pdf.catalog_id = pdf.alloc_id();
    pdf.pages_id = pdf.alloc_id();
    pdf.info_id = pdf.alloc_id();
    for _ in 0..FONT_COUNT {
        let id = pdf.alloc_id();
        pdf.font_ids.push(id);
    }
    for _ in 0..decoded_images.len() {
        let id = pdf.alloc_id();
        pdf.image_ids.push(id);
    }

    let page_w_pt = (theme.slide_w as f32 / EMU_PER_PT).round() as u32;
    let page_h_pt = (theme.slide_h as f32 / EMU_PER_PT).round() as u32;

    let imgs = Imgs {
        by_src: &by_src,
        metas: &metas,
        logo_key: logo_key.as_deref(),
    };

    // Load TTF metrics + (optional) CJK fallback once per render and share
    // across slides.
    let fonts = PdfFonts::load_with_options(font_options)?;

    // Allocate Type0 font ids for the optional CJK face. The first
    // FACE_COUNT ids were allocated earlier; any extras come now.
    for _ in FACE_COUNT..fonts.face_count() {
        let id = pdf.alloc_id();
        pdf.font_ids.push(id);
    }

    pdf.write_header();
    pdf.write_info();
    // Fonts are written at the very end so the per-face glyph-use set is
    // complete and the subsetter can drop unused glyphs. The Type0 object
    // IDs were reserved upfront so page content can reference them.
    pdf.used_glyphs = vec![std::collections::HashSet::new(); fonts.face_count()];
    pdf.write_images(&decoded_images);

    let mut all_links: Vec<Vec<LinkRect>> = Vec::with_capacity(slides.len());
    let mut all_contents: Vec<Vec<u8>> = Vec::with_capacity(slides.len());
    for (i, slide) in slides.iter().enumerate() {
        let num = i + 1;
        let mut sr = SlideRenderer::new(theme, layout, page_w_pt, page_h_pt, &fonts);
        sr.render(slide, num, slides.len(), deck_title, &imgs);
        let (content, links, slide_glyphs) = sr.finish();
        all_contents.push(content);
        all_links.push(links);
        for (face_idx, set) in slide_glyphs.into_iter().enumerate() {
            pdf.used_glyphs[face_idx].extend(set);
        }
    }

    if let Some(n_per) = handout {
        if !matches!(n_per, 2 | 4 | 6) {
            anyhow::bail!(
                "--handout supports 2, 4, or 6 slides per page (got {})",
                n_per
            );
        }
        write_handout(&mut pdf, &all_contents, page_w_pt, page_h_pt, n_per, &fonts)?;
    } else if with_notes {
        let notes: Vec<Option<&str>> = slides.iter().map(|s| s.notes.as_deref()).collect();
        write_notes_pages(
            &mut pdf,
            &all_contents,
            &notes,
            page_w_pt,
            page_h_pt,
            notes_page_size,
            notes_layout,
            &fonts,
        )?;
    } else {
        write_slide_pages(
            &mut pdf,
            &all_contents,
            &all_links,
            page_w_pt,
            page_h_pt,
            transition,
            transition_dur,
        );
    }

    // Glyph 0 (.notdef) must always be present in a subset — PDF spec.
    for set in &mut pdf.used_glyphs {
        set.insert(0);
    }
    // All text has been emitted; now we know exactly which glyphs each
    // face touches and can write subset fonts + CIDToGIDMap.
    let used = std::mem::take(&mut pdf.used_glyphs);
    pdf.write_fonts(&fonts, &used);

    pdf.write_catalog();
    pdf.write_xref_and_trailer();
    Ok(pdf.buf)
}

fn write_slide_pages(
    pdf: &mut PdfWriter,
    all_contents: &[Vec<u8>],
    all_links: &[Vec<LinkRect>],
    page_w_pt: u32,
    page_h_pt: u32,
    transition: Option<&str>,
    transition_dur: f32,
) {
    let n = all_contents.len();
    let mut slide_page_ids: Vec<u32> = (0..n).map(|_| pdf.alloc_id()).collect();
    let slide_content_ids: Vec<u32> = (0..n).map(|_| pdf.alloc_id()).collect();
    let mut slide_annot_ids: Vec<Vec<u32>> = Vec::with_capacity(n);
    for links in all_links {
        let ids: Vec<u32> = (0..links.len()).map(|_| pdf.alloc_id()).collect();
        slide_annot_ids.push(ids);
    }
    let trans_dict = pdf_transition_dict(transition, transition_dur);
    for i in 0..n {
        let compressed = deflate(&all_contents[i]);
        pdf.write_compressed_stream(slide_content_ids[i], &compressed);
        for (link, annot_id) in all_links[i].iter().zip(slide_annot_ids[i].iter()) {
            pdf.write_link_annot(*annot_id, link);
        }
        pdf.write_page_with_trans(
            slide_page_ids[i],
            slide_content_ids[i],
            page_w_pt,
            page_h_pt,
            &slide_annot_ids[i],
            &trans_dict,
        );
    }
    pdf.write_pages_tree(&slide_page_ids);
    let _ = &mut slide_page_ids;
}

fn pdf_transition_dict(kind: Option<&str>, duration_s: f32) -> String {
    let Some(kind) = kind else {
        return String::new();
    };
    let kind = kind.to_ascii_lowercase();
    if kind.is_empty() || kind == "none" {
        return String::new();
    }
    let s = match kind.as_str() {
        "fade" => "/Fade",
        "push" => "/Push",
        "wipe" => "/Wipe",
        "cover" => "/Cover",
        "split" => "/Split",
        _ => return String::new(),
    };
    let d = duration_s.clamp(0.05, 5.0);
    format!(" /Trans << /Type /Trans /S {s} /D {d:.3} >>")
}

fn write_handout(
    pdf: &mut PdfWriter,
    all_contents: &[Vec<u8>],
    slide_w_pt: u32,
    slide_h_pt: u32,
    n_per_page: u32,
    fonts: &PdfFonts,
) -> Result<()> {
    // A4 portrait, in points (1 in = 72 pt).
    let page_w: f32 = 595.0;
    let page_h: f32 = 842.0;
    let margin: f32 = 36.0;
    let gutter: f32 = 18.0;

    // Layout grid: cols × rows.
    let (cols, rows) = match n_per_page {
        2 => (1u32, 2u32),
        4 => (2, 2),
        6 => (2, 3),
        _ => unreachable!(),
    };

    let caption_gap: f32 = 14.0;
    let usable_w = page_w - 2.0 * margin - gutter * (cols as f32 - 1.0);
    let usable_h =
        page_h - 2.0 * margin - (gutter + caption_gap) * (rows as f32 - 1.0) - caption_gap;
    let max_cell_w = usable_w / cols as f32;
    let max_cell_h = usable_h / rows as f32;

    let scale_x = max_cell_w / slide_w_pt as f32;
    let scale_y = max_cell_h / slide_h_pt as f32;
    let scale = scale_x.min(scale_y);
    let thumb_w = scale * slide_w_pt as f32;
    let thumb_h = scale * slide_h_pt as f32;
    let cell_w = thumb_w;
    let cell_h = thumb_h;
    // Center the grid on the page.
    let grid_w = cols as f32 * cell_w + gutter * (cols as f32 - 1.0);
    let grid_h = rows as f32 * cell_h + (gutter + caption_gap) * (rows as f32 - 1.0) + caption_gap;
    let grid_x = (page_w - grid_w) / 2.0;
    let grid_y_top = page_h - (page_h - grid_h) / 2.0;

    // Each slide → one Form XObject.
    let form_ids: Vec<u32> = (0..all_contents.len()).map(|_| pdf.alloc_id()).collect();
    for (i, content) in all_contents.iter().enumerate() {
        pdf.write_form_xobject(form_ids[i], content, slide_w_pt, slide_h_pt);
    }

    // Compose handout pages.
    let per_page = (cols * rows) as usize;
    let n_pages = (all_contents.len() + per_page - 1) / per_page;
    let handout_page_ids: Vec<u32> = (0..n_pages).map(|_| pdf.alloc_id()).collect();
    let handout_content_ids: Vec<u32> = (0..n_pages).map(|_| pdf.alloc_id()).collect();

    for page_idx in 0..n_pages {
        let mut ops = Vec::new();
        let start = page_idx * per_page;
        let end = ((page_idx + 1) * per_page).min(all_contents.len());
        for (slot, slide_idx) in (start..end).enumerate() {
            let col = slot as u32 % cols;
            let row = slot as u32 / cols;
            let cell_x = grid_x + col as f32 * (cell_w + gutter);
            // PDF y is bottom-up; row 0 is the top row.
            let row_top = grid_y_top - row as f32 * (cell_h + gutter + caption_gap);
            let cell_y = row_top - cell_h;
            let tx = cell_x;
            let ty = cell_y;
            let _ = write!(
                &mut ops,
                "q {sx:.5} 0 0 {sy:.5} {tx:.3} {ty:.3} cm /S{i} Do Q\n",
                sx = scale,
                sy = scale,
                tx = tx,
                ty = ty,
                i = slide_idx + 1,
            );
            // Slide number caption below the thumb.
            let caption_y = ty - 12.0;
            if caption_y > 4.0 {
                let label = format!("{} / {}", slide_idx + 1, all_contents.len());
                let label_w = text_width_pt(fonts, &label, 0, 8.0);
                let label_x = tx + (thumb_w - label_w) / 2.0;
                let hex = glyph_hex_string(fonts, &label, 0);
                pdf.record_glyphs(0, &hex);
                let _ = write!(
                    &mut ops,
                    "BT\n/F1 8 Tf\n0.4 0.4 0.4 rg\n{:.3} {:.3} Td\n{} Tj\nET\n",
                    label_x, caption_y, hex,
                );
            }
        }
        let compressed = deflate(&ops);
        pdf.write_compressed_stream(handout_content_ids[page_idx], &compressed);
        pdf.write_handout_page(
            handout_page_ids[page_idx],
            handout_content_ids[page_idx],
            page_w as u32,
            page_h as u32,
            &form_ids,
        );
    }

    pdf.write_pages_tree(&handout_page_ids);
    Ok(())
}

/// Per-slide notes pages. By default each output page uses the deck page
/// size/aspect so a widescreen deck gets widescreen notes pages instead of
/// wasting space on A4 portrait. A4 remains available for print workflows.
/// `notes_layout` controls whether the thumbnail and notes sit side by side
/// or stack vertically; auto mode chooses based on the output page aspect.
/// Pages exist for every slide; slides with no notes get a friendly
/// `— no notes —` placeholder so the page count matches the deck.
fn write_notes_pages(
    pdf: &mut PdfWriter,
    all_contents: &[Vec<u8>],
    notes: &[Option<&str>],
    slide_w_pt: u32,
    slide_h_pt: u32,
    notes_page_size: NotesPageSize,
    notes_layout: NotesLayout,
    fonts: &PdfFonts,
) -> Result<()> {
    let (page_w, page_h) = match notes_page_size {
        NotesPageSize::Slide => (slide_w_pt as f32, slide_h_pt as f32),
        NotesPageSize::A4 => (595.0, 842.0),
    };
    let margin: f32 = page_w.min(page_h) * 0.055;
    let margin = margin.clamp(24.0, 42.0);
    let header_h: f32 = 16.0;
    let gap: f32 = (page_w.min(page_h) * 0.035).clamp(14.0, 26.0);
    let content_top = page_h - margin - header_h - gap;
    let content_bottom = margin;
    let content_h = (content_top - content_bottom).max(80.0);
    let side_by_side = match notes_layout {
        NotesLayout::Auto => page_w > page_h * 1.15,
        NotesLayout::SideBySide => true,
        NotesLayout::Below => false,
    };

    let (scale, thumb_x, thumb_y, notes_left, notes_right, notes_top, notes_bottom, divider) =
        if side_by_side {
            let thumb_max_w = (page_w - 2.0 * margin - gap) * 0.58;
            let thumb_max_h = content_h;
            let scale = (thumb_max_w / slide_w_pt as f32).min(thumb_max_h / slide_h_pt as f32);
            let thumb_w = scale * slide_w_pt as f32;
            let thumb_h = scale * slide_h_pt as f32;
            let thumb_x = margin;
            let thumb_y = content_bottom + (content_h - thumb_h) / 2.0;
            let notes_left = thumb_x + thumb_w + gap;
            let notes_right = page_w - margin;
            (
                scale,
                thumb_x,
                thumb_y,
                notes_left,
                notes_right,
                content_top,
                content_bottom,
                NotesDivider::Vertical {
                    x: notes_left - gap / 2.0,
                    y: content_bottom,
                    h: content_h,
                },
            )
        } else {
            let thumb_max_w = page_w - 2.0 * margin;
            let thumb_max_h = content_h * 0.48;
            let scale = (thumb_max_w / slide_w_pt as f32).min(thumb_max_h / slide_h_pt as f32);
            let thumb_w = scale * slide_w_pt as f32;
            let thumb_h = scale * slide_h_pt as f32;
            let thumb_x = (page_w - thumb_w) / 2.0;
            let thumb_y = content_top - thumb_h;
            let notes_top = thumb_y - gap;
            (
                scale,
                thumb_x,
                thumb_y,
                margin,
                page_w - margin,
                notes_top,
                content_bottom,
                NotesDivider::Horizontal {
                    x: margin,
                    y: notes_top + gap * 0.35,
                    w: page_w - 2.0 * margin,
                },
            )
        };

    let notes_w_pt = (notes_right - notes_left).max(80.0);
    let notes_size_pt: f32 = 10.0;
    let notes_line_h: f32 = notes_size_pt * 1.4;

    let n = all_contents.len();
    let form_ids: Vec<u32> = (0..n).map(|_| pdf.alloc_id()).collect();
    for (i, content) in all_contents.iter().enumerate() {
        pdf.write_form_xobject(form_ids[i], content, slide_w_pt, slide_h_pt);
    }
    let page_ids: Vec<u32> = (0..n).map(|_| pdf.alloc_id()).collect();
    let content_ids: Vec<u32> = (0..n).map(|_| pdf.alloc_id()).collect();

    for i in 0..n {
        let mut ops = Vec::new();

        // Page header: "Slide N / total"
        let header = format!("Slide {} / {}", i + 1, n);
        let header_hex = glyph_hex_string(fonts, &header, 0);
        pdf.record_glyphs(0, &header_hex);
        let _ = write!(
            &mut ops,
            "BT\n/F1 11 Tf\n0.4 0.4 0.4 rg\n{:.3} {:.3} Td\n{} Tj\nET\n",
            margin,
            page_h - margin,
            header_hex,
        );

        // Slide thumbnail.
        let _ = write!(
            &mut ops,
            "q {sx:.5} 0 0 {sy:.5} {tx:.3} {ty:.3} cm /S{idx} Do Q\n",
            sx = scale,
            sy = scale,
            tx = thumb_x,
            ty = thumb_y,
            idx = i + 1,
        );

        match divider {
            NotesDivider::Horizontal { x, y, w } => {
                let _ = write!(
                    &mut ops,
                    "0.85 0.89 0.94 rg\n{x:.3} {y:.3} {w:.3} 0.8 re f\n",
                    x = x,
                    y = y,
                    w = w,
                );
            }
            NotesDivider::Vertical { x, y, h } => {
                let _ = write!(
                    &mut ops,
                    "0.85 0.89 0.94 rg\n{x:.3} {y:.3} 0.8 {h:.3} re f\n",
                    x = x,
                    y = y,
                    h = h,
                );
            }
        }

        // Notes text — word-wrapped.
        let body = notes[i].unwrap_or("— no notes —");
        let lines = wrap_text_simple(
            fonts,
            body,
            0, // SansRegular
            notes_size_pt,
            notes_w_pt,
        );
        let mut y = notes_top - notes_line_h;
        for line in lines {
            if y < notes_bottom {
                break;
            }
            let hex = glyph_hex_string(fonts, &line, 0);
            pdf.record_glyphs(0, &hex);
            let _ = write!(
                &mut ops,
                "BT\n/F1 {sz:.2} Tf\n0.12 0.16 0.23 rg\n{x:.3} {y:.3} Td\n{hex} Tj\nET\n",
                sz = notes_size_pt,
                x = notes_left,
                y = y,
            );
            y -= notes_line_h;
        }

        let compressed = deflate(&ops);
        pdf.write_compressed_stream(content_ids[i], &compressed);
        pdf.write_handout_page(
            page_ids[i],
            content_ids[i],
            page_w as u32,
            page_h as u32,
            &form_ids,
        );
    }

    pdf.write_pages_tree(&page_ids);
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum NotesDivider {
    Horizontal { x: f32, y: f32, w: f32 },
    Vertical { x: f32, y: f32, h: f32 },
}

// ---------------------------------------------------------------------------
// Image collection / decoding
// ---------------------------------------------------------------------------

struct DecodedImage {
    width: u32,
    height: u32,
    /// "DCTDecode" (JPEG raw) or "FlateDecode" (raw RGB pixels, deflated).
    filter: &'static str,
    data: Vec<u8>,
    colorspace: &'static str,
    bpc: u8,
}

fn decode_image(m: &ImageMeta) -> Result<DecodedImage> {
    match m.ext {
        "jpeg" => Ok(DecodedImage {
            width: m.width,
            height: m.height,
            filter: "DCTDecode",
            data: m.bytes.clone(),
            colorspace: "DeviceRGB",
            bpc: 8,
        }),
        "png" => decode_png(m).context("decode PNG for PDF embed"),
        _ => anyhow::bail!("unsupported image format for PDF: {}", m.ext),
    }
}

fn decode_png(m: &ImageMeta) -> Result<DecodedImage> {
    let bytes = &m.bytes;
    if bytes.len() < 8 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        anyhow::bail!("not a PNG");
    }
    let mut idat = Vec::new();
    let mut width = 0u32;
    let mut height = 0u32;
    let mut bit_depth = 8u8;
    let mut color_type = 2u8;
    let mut palette: Option<Vec<[u8; 3]>> = None;
    let mut i = 8usize;
    while i + 8 <= bytes.len() {
        let len = u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]) as usize;
        let kind = &bytes[i + 4..i + 8];
        let data_start = i + 8;
        let data_end = data_start + len;
        if data_end > bytes.len() {
            anyhow::bail!("PNG chunk truncated");
        }
        let data = &bytes[data_start..data_end];
        match kind {
            b"IHDR" => {
                width = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
                height = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
                bit_depth = data[8];
                color_type = data[9];
            }
            b"PLTE" => {
                let mut p = Vec::new();
                for c in data.chunks(3) {
                    if c.len() == 3 {
                        p.push([c[0], c[1], c[2]]);
                    }
                }
                palette = Some(p);
            }
            b"IDAT" => {
                idat.extend_from_slice(data);
            }
            b"IEND" => break,
            _ => {}
        }
        i = data_end + 4; // skip CRC
    }
    if bit_depth != 8 {
        anyhow::bail!(
            "PNG bit depth {} not supported (PDF embed expects 8)",
            bit_depth
        );
    }
    let bpp = match color_type {
        0 => 1, // grayscale
        2 => 3, // RGB
        3 => 1, // palette index
        4 => 2, // grayscale + alpha
        6 => 4, // RGBA
        other => anyhow::bail!("PNG color type {} not supported", other),
    };
    let raw = inflate(&idat).context("inflate IDAT")?;
    let stride = (width as usize) * bpp;
    if raw.len() != (stride + 1) * height as usize {
        anyhow::bail!(
            "PNG data length mismatch: got {}, expected {}",
            raw.len(),
            (stride + 1) * height as usize,
        );
    }
    let mut unfiltered = vec![0u8; stride * height as usize];
    for row in 0..height as usize {
        let filter = raw[row * (stride + 1)];
        let src = &raw[row * (stride + 1) + 1..(row + 1) * (stride + 1)];
        let dst_start = row * stride;
        let prev_row_start = if row > 0 { (row - 1) * stride } else { 0 };
        for col in 0..stride {
            let left = if col >= bpp {
                unfiltered[dst_start + col - bpp]
            } else {
                0
            };
            let up = if row > 0 {
                unfiltered[prev_row_start + col]
            } else {
                0
            };
            let up_left = if row > 0 && col >= bpp {
                unfiltered[prev_row_start + col - bpp]
            } else {
                0
            };
            let raw_byte = src[col];
            let value = match filter {
                0 => raw_byte,
                1 => raw_byte.wrapping_add(left),
                2 => raw_byte.wrapping_add(up),
                3 => raw_byte.wrapping_add(((left as u16 + up as u16) / 2) as u8),
                4 => raw_byte.wrapping_add(paeth(left, up, up_left)),
                _ => anyhow::bail!("PNG filter {} unsupported", filter),
            };
            unfiltered[dst_start + col] = value;
        }
    }
    // Convert to RGB (drop alpha by compositing on white).
    let rgb = match color_type {
        2 => unfiltered, // already RGB
        6 => {
            let mut out = Vec::with_capacity((width * height * 3) as usize);
            for px in unfiltered.chunks_exact(4) {
                let a = px[3] as u32;
                let blend = |c: u8| -> u8 {
                    let v = (c as u32) * a + 255 * (255 - a);
                    (v / 255) as u8
                };
                out.push(blend(px[0]));
                out.push(blend(px[1]));
                out.push(blend(px[2]));
            }
            out
        }
        0 => {
            let mut out = Vec::with_capacity((width * height * 3) as usize);
            for &g in &unfiltered {
                out.push(g);
                out.push(g);
                out.push(g);
            }
            out
        }
        4 => {
            let mut out = Vec::with_capacity((width * height * 3) as usize);
            for px in unfiltered.chunks_exact(2) {
                let g = px[0];
                let a = px[1] as u32;
                let blend = |c: u8| -> u8 {
                    let v = (c as u32) * a + 255 * (255 - a);
                    (v / 255) as u8
                };
                out.push(blend(g));
                out.push(blend(g));
                out.push(blend(g));
            }
            out
        }
        3 => {
            let pal = palette.unwrap_or_default();
            let mut out = Vec::with_capacity((width * height * 3) as usize);
            for &idx in &unfiltered {
                let c = pal.get(idx as usize).copied().unwrap_or([0, 0, 0]);
                out.push(c[0]);
                out.push(c[1]);
                out.push(c[2]);
            }
            out
        }
        _ => unreachable!(),
    };
    let deflated = deflate(&rgb);
    Ok(DecodedImage {
        width,
        height,
        filter: "FlateDecode",
        data: deflated,
        colorspace: "DeviceRGB",
        bpc: 8,
    })
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i32 + b as i32 - c as i32;
    let pa = (p - a as i32).abs();
    let pb = (p - b as i32).abs();
    let pc = (p - c as i32).abs();
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

fn inflate(data: &[u8]) -> Result<Vec<u8>> {
    use flate2::read::ZlibDecoder;
    use std::io::Read;
    let mut decoder = ZlibDecoder::new(data);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out)?;
    Ok(out)
}

fn deflate(data: &[u8]) -> Vec<u8> {
    let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
    e.write_all(data).expect("deflate");
    e.finish().expect("deflate finish")
}

/// Resolve a per-slide background token (`#hex` or palette keyword) to a hex
/// string without the leading `#`.
fn resolve_bg_color(token: &str, theme: &Theme) -> String {
    if let Some(hex) = token.strip_prefix('#') {
        return hex.to_string();
    }
    match token.to_ascii_lowercase().as_str() {
        "accent" => theme.accent.clone(),
        "section" => theme.section_bg.clone(),
        "dark" => "0F172A".to_string(),
        "light" => "FFFFFF".to_string(),
        _ => theme.bg.clone(),
    }
}

fn collect_block_images(
    blocks: &[Block],
    base_dir: &Path,
    metas: &mut Vec<ImageMeta>,
    by_src: &mut HashMap<String, usize>,
) -> Result<()> {
    for b in blocks {
        match b {
            Block::Image { src, .. } => {
                if by_src.contains_key(src) {
                    continue;
                }
                let meta = image::load_any_or_placeholder(base_dir, src);
                by_src.insert(src.clone(), metas.len());
                metas.push(meta);
            }
            Block::Columns { left, right } => {
                collect_block_images(left, base_dir, metas, by_src)?;
                collect_block_images(right, base_dir, metas, by_src)?;
            }
            _ => {}
        }
    }
    Ok(())
}

struct Imgs<'a> {
    by_src: &'a HashMap<String, usize>,
    metas: &'a [ImageMeta],
    logo_key: Option<&'a str>,
}

impl<'a> Imgs<'a> {
    fn dims(&self, src: &str) -> Option<(u32, u32)> {
        self.by_src
            .get(src)
            .map(|i| (self.metas[*i].width, self.metas[*i].height))
    }
    fn index(&self, src: &str) -> Option<usize> {
        self.by_src.get(src).copied()
    }
    fn logo(&self) -> Option<(&str, u32, u32)> {
        let k = self.logo_key?;
        let i = *self.by_src.get(k)?;
        Some((k, self.metas[i].width, self.metas[i].height))
    }
}

// ---------------------------------------------------------------------------
// Fonts
// ---------------------------------------------------------------------------

use crate::font::{FaceKind, FaceMetrics, PdfFonts, FACE_COUNT};

const FONT_COUNT: usize = FACE_COUNT;
// Slot indices reused throughout the renderer. Map to `FaceKind`'s discriminant.
const FONT_HELV: usize = 0; // DejaVu Sans Regular
const FONT_HELV_BOLD: usize = 1; // DejaVu Sans Bold
const FONT_HELV_OBL: usize = 2; // DejaVu Sans Oblique
const FONT_HELV_BOLD_OBL: usize = 3; // DejaVu Sans Bold Oblique
const FONT_COUR: usize = 4; // DejaVu Sans Mono
const FONT_COUR_BOLD: usize = 4; // Same as Mono; bold mono not bundled.

fn face_for_index(idx: usize) -> FaceKind {
    match idx {
        0 => FaceKind::SansRegular,
        1 => FaceKind::SansBold,
        2 => FaceKind::SansOblique,
        3 => FaceKind::SansBoldOblique,
        _ => FaceKind::Mono,
    }
}

fn font_index(bold: bool, italic: bool, mono: bool) -> usize {
    match (mono, bold, italic) {
        (true, _, _) => FONT_COUR,
        (false, false, false) => FONT_HELV,
        (false, true, false) => FONT_HELV_BOLD,
        (false, false, true) => FONT_HELV_OBL,
        (false, true, true) => FONT_HELV_BOLD_OBL,
    }
}

/// Compute text width in points using the embedded TTF metrics for the
/// chosen face. Sums glyph advances (in font units) and converts to points
/// via `units_per_em`. Codepoints with no glyph in the font contribute the
/// .notdef advance (typically a small box), matching what the PDF will
/// actually render.
/// Total advance of one wrapped line of runs, in EMU, honouring each run's
/// bold/italic. Used to offset table cells for center/right alignment.
fn runs_width_emu(fonts: &PdfFonts, line: &[Run], size_centipt: u32, base_bold: bool) -> u32 {
    let size_pt = size_centipt as f32 / 100.0;
    let w: f32 = line
        .iter()
        .map(|r| {
            let idx = font_index(base_bold || r.bold, r.italic, r.code);
            text_width_pt(fonts, &r.text, idx, size_pt)
        })
        .sum();
    (w * EMU_PER_PT) as u32
}

/// Left x (EMU) at which to start a cell line of width `line_w` so it sits
/// left / centre / right within a column, per the GFM alignment.
fn aligned_cell_x(
    align: Option<crate::ir::ColumnAlign>,
    col_left: u32,
    col_w: u32,
    pad_x: u32,
    line_w: u32,
) -> u32 {
    match align {
        Some(crate::ir::ColumnAlign::Center) => col_left + col_w.saturating_sub(line_w) / 2,
        Some(crate::ir::ColumnAlign::Right) => {
            col_left + col_w.saturating_sub(pad_x).saturating_sub(line_w)
        }
        _ => col_left + pad_x,
    }
}

fn text_width_pt(fonts: &PdfFonts, text: &str, font_idx: usize, size_pt: f32) -> f32 {
    let mut total = 0.0_f32;
    for c in text.chars() {
        let (face_idx, gid) = pick_face(fonts, font_idx, c);
        let face = &fonts.metrics[face_idx];
        total += face.glyph_width_pt(gid, size_pt);
    }
    total
}

/// Choose which face actually has a glyph for `c`. If the primary face
/// (Latin / Mono) covers it, use that. Otherwise try the optional CJK
/// fallback. Returns (face_idx, glyph_id); on total miss, returns the
/// primary face with .notdef so the caller still emits *something*.
fn pick_face(fonts: &PdfFonts, primary: usize, c: char) -> (usize, u16) {
    fonts.face_for_char(primary, c).unwrap_or((primary, 0))
}

/// PDF `/ToUnicode` CMap stream body. Maps each glyph ID present in the
/// font back to the (first) Unicode codepoint that resolves to it, so a
/// reader copying text from the PDF gets back real characters rather than
/// glyph indices. Non-BMP codepoints are emitted as UTF-16BE surrogate
/// pairs inside the `<hex>` quads.
fn build_cmap_body(pairs: &[(u16, char)], font_name: &str) -> String {
    let mut s = String::with_capacity(pairs.len() * 24);
    s.push_str(
        "/CIDInit /ProcSet findresource begin\n\
         12 dict begin\n\
         begincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n",
    );
    s.push_str(&format!("/CMapName /{}-UCS def\n", font_name));
    s.push_str(
        "/CMapType 2 def\n\
         1 begincodespacerange\n\
         <0000> <FFFF>\n\
         endcodespacerange\n",
    );
    for chunk in pairs.chunks(100) {
        s.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for (gid, c) in chunk {
            s.push_str(&format!("<{:04X}> {}\n", gid, utf16be_hex(*c)));
        }
        s.push_str("endbfchar\n");
    }
    s.push_str(
        "endcmap\n\
         CMapName currentdict /CMap defineresource pop\n\
         end\nend\n",
    );
    s
}

/// `/ToUnicode` CMap for a face's full glyph set. Reads every (gid → cp)
/// mapping the cmap exposes and emits a single chunked PDF CMap.
fn build_tounicode_cmap(face: &FaceMetrics, ttf: &[u8], font_name: &str) -> String {
    build_cmap_body(&face.cid_to_unicode(ttf), font_name)
}

/// Encode a single Unicode codepoint as a PDF hex literal in UTF-16BE.
/// BMP characters are one 4-hex quad; supplementary-plane characters
/// (≥ U+10000) need a surrogate pair (two quads).
fn utf16be_hex(c: char) -> String {
    let cp = c as u32;
    if cp <= 0xFFFF {
        format!("<{:04X}>", cp)
    } else {
        let v = cp - 0x10000;
        let hi = 0xD800 + (v >> 10);
        let lo = 0xDC00 + (v & 0x3FF);
        format!("<{:04X}{:04X}>", hi, lo)
    }
}

/// Build the `<hexhex…>` hex string used inside a PDF `Tj` operator when
/// the font is Identity-H encoded. Each codepoint becomes 4 hex characters
/// (the glyph ID in big-endian).
///
/// **Single-face only** — assumes the primary face has glyphs for the whole
/// text. Use [`glyph_hex_runs`] when text might need the CJK fallback
/// (text_line / draw_runs_at — anything authored by the user).
fn glyph_hex_string(fonts: &PdfFonts, text: &str, font_idx: usize) -> String {
    let face = &fonts.metrics[font_idx];
    let mut out = String::with_capacity(text.len() * 4 + 2);
    out.push('<');
    for c in text.chars() {
        let gid = face.glyph_for_char(&fonts.bytes[font_idx], c).unwrap_or(0);
        out.push_str(&format!("{:04X}", gid));
    }
    out.push('>');
    out
}

/// Parse glyph IDs out of a `<HHHH...>` hex literal and add them to `set`.
/// Used to feed the per-face subsetter as we emit text runs.
fn record_glyphs_from_hex(set: &mut std::collections::HashSet<u16>, hex: &str) {
    let stripped = hex.trim_start_matches('<').trim_end_matches('>');
    let mut bytes = stripped.as_bytes();
    while bytes.len() >= 4 {
        let s = std::str::from_utf8(&bytes[..4]).unwrap_or("0000");
        if let Ok(gid) = u16::from_str_radix(s, 16) {
            set.insert(gid);
        }
        bytes = &bytes[4..];
    }
}

/// Subset a TTF to just the glyph IDs in `used`. The returned remapper
/// maps original glyph IDs to the subset's dense CIDs (0..N) — md2any
/// keeps the original IDs in text streams and uses a /CIDToGIDMap entry
/// on the font dictionary to bridge the two, so the remapper is only
/// needed by the /ToUnicode CMap builder. Returns `Err` for fonts the
/// subsetter can't handle (e.g. TTC files with no face at index 0);
/// callers should fall back to the original bytes + an identity remapper.
/// True when the sfnt carries PostScript/CFF outlines rather than TrueType
/// (`glyf`) ones. CFF-flavoured OpenType begins with the `OTTO` magic; plain
/// TrueType uses `0x00010000` or `true`. This decides the PDF font structure:
/// CFF needs a `CIDFontType0` descendant + `FontFile3`, while `glyf` uses
/// `CIDFontType2` + `FontFile2`. The bundled DejaVu faces are all `glyf`.
fn font_has_cff_outlines(bytes: &[u8]) -> bool {
    bytes.starts_with(b"OTTO")
}

fn subset_font(
    ttf: &[u8],
    used: &std::collections::HashSet<u16>,
) -> anyhow::Result<(Vec<u8>, subsetter::GlyphRemapper)> {
    let mut glyphs: Vec<u16> = used.iter().copied().collect();
    glyphs.sort_unstable();
    let remapper = subsetter::GlyphRemapper::new_from_glyphs_sorted(&glyphs);
    let out = subsetter::subset(ttf, 0, &remapper)
        .map_err(|e| anyhow::anyhow!("subset font: {:?}", e))?;
    Ok((out.to_vec(), remapper))
}

/// Like [`glyph_hex_string`] but splits the text into runs whenever the
/// character would need the CJK fallback face. Returns a sequence of
/// `(face_idx, hex_string, advance_pt)` tuples — the caller emits one
/// `BT … Tj … ET` block per tuple, advancing the x cursor by `advance_pt`
/// between them. When no CJK fallback is loaded (or every character has a
/// glyph in the primary face) the result is a single-element vector.
fn glyph_hex_runs(
    fonts: &PdfFonts,
    text: &str,
    primary: usize,
    size_pt: f32,
) -> Vec<(usize, String, f32)> {
    let mut runs: Vec<(usize, String, f32)> = Vec::new();
    for c in text.chars() {
        let (face_idx, gid) = pick_face(fonts, primary, c);
        let advance = fonts.metrics[face_idx].glyph_width_pt(gid, size_pt);
        let hex_pair = format!("{:04X}", gid);
        match runs.last_mut() {
            Some((f, hex, a)) if *f == face_idx => {
                hex.push_str(&hex_pair);
                *a += advance;
            }
            _ => runs.push((face_idx, hex_pair, advance)),
        }
    }
    // Wrap each run's hex in `<…>` so callers can drop it straight into a
    // `Tj` operator.
    for (_, hex, _) in &mut runs {
        let wrapped = format!("<{}>", hex);
        *hex = wrapped;
    }
    runs
}

/// Map Unicode char to a WinAnsi code, dropping anything we can't represent.
fn unicode_to_winansi(c: char) -> Option<u8> {
    if (c as u32) < 0x80 {
        return Some(c as u8);
    }
    match c {
        '€' => Some(0x80),
        '‚' => Some(0x82),
        'ƒ' => Some(0x83),
        '„' => Some(0x84),
        '…' => Some(0x85),
        '†' => Some(0x86),
        '‡' => Some(0x87),
        'ˆ' => Some(0x88),
        '‰' => Some(0x89),
        'Š' => Some(0x8A),
        '‹' => Some(0x8B),
        'Œ' => Some(0x8C),
        'Ž' => Some(0x8E),
        '‘' => Some(0x91),
        '’' => Some(0x92),
        '“' => Some(0x93),
        '”' => Some(0x94),
        '•' => Some(0x95),
        '–' => Some(0x96),
        '—' => Some(0x97),
        '˜' => Some(0x98),
        '™' => Some(0x99),
        'š' => Some(0x9A),
        '›' => Some(0x9B),
        'œ' => Some(0x9C),
        'ž' => Some(0x9E),
        'Ÿ' => Some(0x9F),
        '·' => Some(0xB7),
        '●' => Some(0x95), // map filled black circle to bullet
        '○' => Some(b'o'),
        '▪' => Some(0x95),
        c if (c as u32) >= 0xA0 && (c as u32) <= 0xFF => Some(c as u8),
        _ => None,
    }
}

fn encode_winansi(s: &str) -> Vec<u8> {
    // Track whether the previous emitted character was a Unicode
    // sub/superscript so we can avoid repeating the `_` / `^` prefix for
    // consecutive members of the same group. Example: `xᵢⱼ` should fall
    // back to `x_ij`, not `x_i_j`.
    let mut prev = SubSuperKind::Plain;

    let mut out = Vec::with_capacity(s.len());
    for c in s.chars() {
        if let Some(b) = unicode_to_winansi(c) {
            out.push(b);
            prev = SubSuperKind::Plain;
        } else if let Some((kind, fallback)) = math_ascii_fallback(c) {
            if kind != SubSuperKind::Plain && kind != prev {
                out.push(match kind {
                    SubSuperKind::Super => b'^',
                    SubSuperKind::Sub => b'_',
                    SubSuperKind::Plain => unreachable!(),
                });
            }
            out.extend_from_slice(fallback.as_bytes());
            prev = kind;
        } else {
            prev = SubSuperKind::Plain;
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq)]
enum SubSuperKind {
    Plain,
    Super,
    Sub,
}

/// PDF fallback for the Unicode glyphs the math translator emits that
/// WinAnsi can't represent (Greek, math operators, sub/superscripts).
/// Returns the ASCII rendering plus a hint about whether the original was a
/// sub/superscript so the caller can prefix `_` or `^` appropriately.
fn math_ascii_fallback(c: char) -> Option<(SubSuperKind, &'static str)> {
    use SubSuperKind::*;
    Some(match c {
        // Lowercase Greek
        'α' => (Plain, "alpha"),
        'β' => (Plain, "beta"),
        'γ' => (Plain, "gamma"),
        'δ' => (Plain, "delta"),
        'ε' => (Plain, "epsilon"),
        'ζ' => (Plain, "zeta"),
        'η' => (Plain, "eta"),
        'θ' => (Plain, "theta"),
        'ι' => (Plain, "iota"),
        'κ' => (Plain, "kappa"),
        'λ' => (Plain, "lambda"),
        'μ' => (Plain, "mu"),
        'ν' => (Plain, "nu"),
        'ξ' => (Plain, "xi"),
        'π' => (Plain, "pi"),
        'ρ' => (Plain, "rho"),
        'σ' => (Plain, "sigma"),
        'ς' => (Plain, "sigma"),
        'τ' => (Plain, "tau"),
        'υ' => (Plain, "upsilon"),
        'φ' => (Plain, "phi"),
        'ϕ' => (Plain, "phi"),
        'ϑ' => (Plain, "theta"),
        'ϖ' => (Plain, "pi"),
        'ϱ' => (Plain, "rho"),
        'χ' => (Plain, "chi"),
        'ψ' => (Plain, "psi"),
        'ω' => (Plain, "omega"),
        // Uppercase Greek
        'Α' => (Plain, "A"),
        'Β' => (Plain, "B"),
        'Γ' => (Plain, "Gamma"),
        'Δ' => (Plain, "Delta"),
        'Ε' => (Plain, "E"),
        'Ζ' => (Plain, "Z"),
        'Η' => (Plain, "H"),
        'Θ' => (Plain, "Theta"),
        'Ι' => (Plain, "I"),
        'Κ' => (Plain, "K"),
        'Λ' => (Plain, "Lambda"),
        'Μ' => (Plain, "M"),
        'Ν' => (Plain, "N"),
        'Ξ' => (Plain, "Xi"),
        'Π' => (Plain, "Pi"),
        'Ρ' => (Plain, "P"),
        'Σ' => (Plain, "Sigma"),
        'Τ' => (Plain, "T"),
        'Υ' => (Plain, "Y"),
        'Φ' => (Plain, "Phi"),
        'Χ' => (Plain, "X"),
        'Ψ' => (Plain, "Psi"),
        'Ω' => (Plain, "Omega"),
        // Big operators
        '∑' => (Plain, "Sum"),
        '∏' => (Plain, "Prod"),
        '∫' => (Plain, "Int"),
        '∮' => (Plain, "Int"),
        '⋃' => (Plain, "Union"),
        '⋂' => (Plain, "Inter"),
        // Relations
        '≤' => (Plain, "<="),
        '≥' => (Plain, ">="),
        '≠' => (Plain, "!="),
        '≈' => (Plain, "~="),
        '≡' => (Plain, "=="),
        '∼' => (Plain, "~"),
        '≃' => (Plain, "~="),
        '∝' => (Plain, "prop"),
        // Arrows
        '→' => (Plain, "->"),
        '←' => (Plain, "<-"),
        '↔' => (Plain, "<->"),
        '⇒' => (Plain, "=>"),
        '⇐' => (Plain, "<="),
        '⇔' => (Plain, "<=>"),
        '↦' => (Plain, "|->"),
        // Operators
        '±' => (Plain, "+/-"),
        '∓' => (Plain, "-/+"),
        '×' => (Plain, "x"),
        '÷' => (Plain, "/"),
        '∪' => (Plain, "U"),
        '∩' => (Plain, "n"),
        '∖' => (Plain, "\\"),
        '∗' => (Plain, "*"),
        '⋆' => (Plain, "*"),
        // Logic / set theory
        '∀' => (Plain, "forall"),
        '∃' => (Plain, "exists"),
        '∄' => (Plain, "!exists"),
        '∈' => (Plain, "in"),
        '∉' => (Plain, "!in"),
        '⊂' => (Plain, "sub"),
        '⊃' => (Plain, "sup"),
        '⊆' => (Plain, "subeq"),
        '⊇' => (Plain, "supeq"),
        '∧' => (Plain, "and"),
        '∨' => (Plain, "or"),
        '¬' => (Plain, "!"),
        // Misc math
        '∞' => (Plain, "inf"),
        '∅' => (Plain, "{}"),
        '∂' => (Plain, "d"),
        '∇' => (Plain, "grad"),
        'ℏ' => (Plain, "hbar"),
        'ℓ' => (Plain, "l"),
        'ℜ' => (Plain, "Re"),
        'ℑ' => (Plain, "Im"),
        'ℵ' => (Plain, "aleph"),
        '…' => (Plain, "..."),
        '⋯' => (Plain, "..."),
        '⋮' => (Plain, ":"),
        '⋱' => (Plain, "..."),
        '√' => (Plain, "sqrt"),
        // Superscript digits + signs (those not in WinAnsi). 1/2/3 are
        // already in WinAnsi so we don't list them here.
        '⁰' => (Super, "0"),
        '⁴' => (Super, "4"),
        '⁵' => (Super, "5"),
        '⁶' => (Super, "6"),
        '⁷' => (Super, "7"),
        '⁸' => (Super, "8"),
        '⁹' => (Super, "9"),
        '⁺' => (Super, "+"),
        '⁻' => (Super, "-"),
        '⁼' => (Super, "="),
        '⁽' => (Super, "("),
        '⁾' => (Super, ")"),
        // Superscript lowercase modifier letters
        'ⁿ' => (Super, "n"),
        'ⁱ' => (Super, "i"),
        'ᵃ' => (Super, "a"),
        'ᵇ' => (Super, "b"),
        'ᶜ' => (Super, "c"),
        'ᵈ' => (Super, "d"),
        'ᵉ' => (Super, "e"),
        'ᶠ' => (Super, "f"),
        'ᵍ' => (Super, "g"),
        'ʰ' => (Super, "h"),
        'ʲ' => (Super, "j"),
        'ᵏ' => (Super, "k"),
        'ˡ' => (Super, "l"),
        'ᵐ' => (Super, "m"),
        'ᵒ' => (Super, "o"),
        'ᵖ' => (Super, "p"),
        'ʳ' => (Super, "r"),
        'ˢ' => (Super, "s"),
        'ᵗ' => (Super, "t"),
        'ᵘ' => (Super, "u"),
        'ᵛ' => (Super, "v"),
        'ʷ' => (Super, "w"),
        'ˣ' => (Super, "x"),
        'ʸ' => (Super, "y"),
        'ᶻ' => (Super, "z"),
        // Superscript uppercase modifier letters
        'ᴬ' => (Super, "A"),
        'ᴮ' => (Super, "B"),
        'ᴰ' => (Super, "D"),
        'ᴱ' => (Super, "E"),
        'ᴳ' => (Super, "G"),
        'ᴴ' => (Super, "H"),
        'ᴵ' => (Super, "I"),
        'ᴶ' => (Super, "J"),
        'ᴷ' => (Super, "K"),
        'ᴸ' => (Super, "L"),
        'ᴹ' => (Super, "M"),
        'ᴺ' => (Super, "N"),
        'ᴼ' => (Super, "O"),
        'ᴾ' => (Super, "P"),
        'ᴿ' => (Super, "R"),
        'ᵀ' => (Super, "T"),
        'ᵁ' => (Super, "U"),
        'ⱽ' => (Super, "V"),
        'ᵂ' => (Super, "W"),
        // Subscript digits + signs
        '₀' => (Sub, "0"),
        '₁' => (Sub, "1"),
        '₂' => (Sub, "2"),
        '₃' => (Sub, "3"),
        '₄' => (Sub, "4"),
        '₅' => (Sub, "5"),
        '₆' => (Sub, "6"),
        '₇' => (Sub, "7"),
        '₈' => (Sub, "8"),
        '₉' => (Sub, "9"),
        '₊' => (Sub, "+"),
        '₋' => (Sub, "-"),
        '₌' => (Sub, "="),
        '₍' => (Sub, "("),
        '₎' => (Sub, ")"),
        // Subscript letters
        'ₐ' => (Sub, "a"),
        'ₑ' => (Sub, "e"),
        'ₕ' => (Sub, "h"),
        'ᵢ' => (Sub, "i"),
        'ⱼ' => (Sub, "j"),
        'ₖ' => (Sub, "k"),
        'ₗ' => (Sub, "l"),
        'ₘ' => (Sub, "m"),
        'ₙ' => (Sub, "n"),
        'ₒ' => (Sub, "o"),
        'ₚ' => (Sub, "p"),
        'ᵣ' => (Sub, "r"),
        'ₛ' => (Sub, "s"),
        'ₜ' => (Sub, "t"),
        'ᵤ' => (Sub, "u"),
        'ᵥ' => (Sub, "v"),
        'ₓ' => (Sub, "x"),
        _ => return None,
    })
}

fn pdf_escape_string(s: &str) -> Vec<u8> {
    let bytes = encode_winansi(s);
    let mut out = Vec::with_capacity(bytes.len() + 8);
    for b in bytes {
        match b {
            b'(' => out.extend_from_slice(b"\\("),
            b')' => out.extend_from_slice(b"\\)"),
            b'\\' => out.extend_from_slice(b"\\\\"),
            0x0A => out.extend_from_slice(b"\\n"),
            0x0D => out.extend_from_slice(b"\\r"),
            b => out.push(b),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// PdfWriter — byte buffer with object positions
// ---------------------------------------------------------------------------

struct PdfWriter {
    buf: Vec<u8>,
    /// objects[i] = byte offset of object (i+1). Length = total objects.
    offsets: Vec<u64>,
    next_id: u32,
    catalog_id: u32,
    pages_id: u32,
    info_id: u32,
    font_ids: Vec<u32>,
    image_ids: Vec<u32>,
    title: String,
    author: String,
    slide_w: u32,
    slide_h: u32,
    /// Per-face glyph IDs touched by anything written so far. Pre-sized to
    /// fonts.face_count() before content is emitted. Drives font
    /// subsetting in [`write_fonts`].
    used_glyphs: Vec<std::collections::HashSet<u16>>,
}

impl PdfWriter {
    fn new(title: &str, author: &str, slide_w: u32, slide_h: u32) -> Self {
        PdfWriter {
            buf: Vec::with_capacity(64 * 1024),
            offsets: Vec::new(),
            next_id: 0,
            catalog_id: 0,
            pages_id: 0,
            info_id: 0,
            font_ids: Vec::new(),
            image_ids: Vec::new(),
            title: title.to_string(),
            author: author.to_string(),
            slide_w,
            slide_h,
            used_glyphs: Vec::new(),
        }
    }

    /// Record one or more glyph IDs (as the 4-hex-char-per-glyph string
    /// found in a `<…>` Tj literal) against `face`.
    fn record_glyphs(&mut self, face: usize, hex: &str) {
        while self.used_glyphs.len() <= face {
            self.used_glyphs.push(std::collections::HashSet::new());
        }
        record_glyphs_from_hex(&mut self.used_glyphs[face], hex);
    }

    fn alloc_id(&mut self) -> u32 {
        self.next_id += 1;
        self.offsets.push(0);
        self.next_id
    }

    fn write_header(&mut self) {
        self.buf.extend_from_slice(b"%PDF-1.7\n");
        // Binary comment marker — high bytes hint to readers that this is binary.
        self.buf.extend_from_slice(b"%\xE2\xE3\xCF\xD3\n");
    }

    fn start_object(&mut self, id: u32) {
        let offset = self.buf.len() as u64;
        self.offsets[(id - 1) as usize] = offset;
        self.buf
            .extend_from_slice(format!("{} 0 obj\n", id).as_bytes());
    }

    fn end_object(&mut self) {
        self.buf.extend_from_slice(b"\nendobj\n");
    }

    fn write_info(&mut self) {
        self.start_object(self.info_id);
        self.buf.extend_from_slice(b"<<\n");
        self.buf.extend_from_slice(b"/Title (");
        self.buf
            .extend_from_slice(&pdf_escape_string(&self.title.clone()));
        self.buf.extend_from_slice(b")\n");
        self.buf.extend_from_slice(b"/Author (");
        self.buf
            .extend_from_slice(&pdf_escape_string(&self.author.clone()));
        self.buf.extend_from_slice(b")\n");
        self.buf.extend_from_slice(b"/Producer (md2any)\n");
        self.buf.extend_from_slice(b"/Creator (md2any)\n");
        self.buf.extend_from_slice(b">>");
        self.end_object();
    }

    /// Emit a Type0 / CIDFontType2 setup for every bundled DejaVu face.
    ///
    /// For each face we allocate four PDF objects:
    ///   1. **Type0 font dict** — what page resources reference as /F1..N.
    ///   2. **CIDFontType2** — the descendant CID font that owns widths +
    ///      points at the font program.
    ///   3. **FontDescriptor** — global metrics + FontFile2 pointer.
    ///   4. **FontFile2 stream** — the raw TTF bytes, flate-compressed.
    ///
    /// Text in page content streams is written as Identity-H encoded hex
    /// (`<00480065006C006C006F>` = "Hello"), which means CIDs equal glyph
    /// IDs and the writer can hand glyph IDs straight to the PDF.
    /// Writes a Type0 / CIDFontType2 quartet per face, subsetting each
    /// font to only the glyphs collected in `used`. The subset has dense
    /// new GIDs (0..N) but we emit a `/CIDToGIDMap` stream that maps the
    /// original glyph IDs (what page content streams reference) to the new
    /// IDs, so we don't have to rewrite any text streams.
    fn write_fonts(&mut self, fonts: &PdfFonts, used: &[std::collections::HashSet<u16>]) {
        for i in 0..fonts.face_count() {
            let face_metrics = &fonts.metrics[i];
            let is_cff = font_has_cff_outlines(&fonts.bytes[i]);
            let cidfont_id = self.alloc_id();
            let descriptor_id = self.alloc_id();
            let fontfile_id = self.alloc_id();
            let tounicode_id = self.alloc_id();
            // CIDToGIDMap is a CIDFontType2-only construct; CFF descendants
            // (CIDFontType0) resolve CID→glyph through the font itself, so we
            // neither allocate nor emit the stream for them. Allocating it
            // only on the glyf path keeps the default (DejaVu) output and
            // object numbering byte-for-byte unchanged.
            let cidtogid_id = if is_cff { None } else { Some(self.alloc_id()) };
            // glyf faces are subset to the glyphs actually used (dense GIDs +
            // a CIDToGIDMap translating original→subset). For CFF/OpenType we
            // embed the full program and keep CID == original GID: the
            // Identity-H text streams already carry original GIDs and
            // CIDFontType0 offers no map to translate them, so subsetting
            // would require rewriting every Tj literal. The trade-off is a
            // larger embed for custom OTF faces only.
            let (subset, remapper): (Vec<u8>, Option<subsetter::GlyphRemapper>) = if is_cff {
                (fonts.bytes[i].clone(), None)
            } else {
                match subset_font(&fonts.bytes[i], &used[i]) {
                    Ok((bytes, remapper)) => (bytes, Some(remapper)),
                    Err(_) => {
                        let all: Vec<u16> = (0..face_metrics.num_glyphs).collect();
                        let identity = subsetter::GlyphRemapper::new_from_glyphs_sorted(&all);
                        (fonts.bytes[i].clone(), Some(identity))
                    }
                }
            };
            let ttf_bytes: &[u8] = &subset;
            let ps_name = fonts.names[i].as_str();
            let kind = if i < FACE_COUNT {
                Some(face_for_index(i))
            } else {
                None
            };

            // 1) Type0 wrapper.
            self.start_object(self.font_ids[i]);
            let body = format!(
                "<< /Type /Font /Subtype /Type0 /BaseFont /{name} \
                 /Encoding /Identity-H /DescendantFonts [{cid} 0 R] \
                 /ToUnicode {tu} 0 R >>",
                name = ps_name,
                cid = cidfont_id,
                tu = tounicode_id,
            );
            self.buf.extend_from_slice(body.as_bytes());
            self.end_object();

            // 2) CIDFontType2 with per-glyph width array.
            let scale = 1000.0 / face_metrics.units_per_em as f32;
            // /W is CID-indexed; our CIDs equal *original* GIDs (since the
            // CIDToGIDMap below translates them to subset GIDs), so we need
            // widths for every original GID that any Tj literal references.
            // We use the explicit `c [w]` form, one entry per used GID.
            let mut used_sorted: Vec<u16> = used[i].iter().copied().collect();
            used_sorted.sort_unstable();
            let mut widths = String::with_capacity(used_sorted.len() * 12);
            widths.push('[');
            for orig_gid in &used_sorted {
                let w = (face_metrics.glyph_width(*orig_gid) as f32 * scale).round() as i32;
                widths.push_str(&format!("{gid} [{w}] ", gid = orig_gid, w = w));
            }
            widths.push(']');
            self.start_object(cidfont_id);
            let cid_subtype = if is_cff {
                "CIDFontType0"
            } else {
                "CIDFontType2"
            };
            let cidtogid_entry = match cidtogid_id {
                Some(id) => format!(" /CIDToGIDMap {id} 0 R"),
                None => String::new(),
            };
            let cid_body = format!(
                "<< /Type /Font /Subtype /{subtype} /BaseFont /{name} \
                 /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
                 /FontDescriptor {desc} 0 R{ctgm} /W {w} >>",
                subtype = cid_subtype,
                name = ps_name,
                desc = descriptor_id,
                ctgm = cidtogid_entry,
                w = widths,
            );
            self.buf.extend_from_slice(cid_body.as_bytes());
            self.end_object();

            // 3) FontDescriptor.
            let bbox = (
                (face_metrics.bbox.0 as f32 * scale).round() as i32,
                (face_metrics.bbox.1 as f32 * scale).round() as i32,
                (face_metrics.bbox.2 as f32 * scale).round() as i32,
                (face_metrics.bbox.3 as f32 * scale).round() as i32,
            );
            let ascent = (face_metrics.ascent as f32 * scale).round() as i32;
            let descent = (face_metrics.descent as f32 * scale).round() as i32;
            let cap_height = (face_metrics.cap_height as f32 * scale).round() as i32;
            // Flags: 4 = monospace, 64 = italic, 32 = nonsymbolic (general).
            let mut flags = 32u32;
            if matches!(kind, Some(FaceKind::Mono)) {
                flags |= 1; // FixedPitch
            }
            if matches!(
                kind,
                Some(FaceKind::SansOblique | FaceKind::SansBoldOblique)
            ) {
                flags |= 64; // Italic
            }
            self.start_object(descriptor_id);
            // CFF outlines are referenced via FontFile3; TrueType via FontFile2.
            let fontfile_key = if is_cff { "FontFile3" } else { "FontFile2" };
            let desc_body = format!(
                "<< /Type /FontDescriptor /FontName /{name} /Flags {flags} \
                 /FontBBox [{bx0} {by0} {bx1} {by1}] /ItalicAngle {ia} \
                 /Ascent {asc} /Descent {dsc} /CapHeight {cap} /StemV 80 \
                 /{fontfile_key} {file} 0 R >>",
                name = ps_name,
                flags = flags,
                bx0 = bbox.0,
                by0 = bbox.1,
                bx1 = bbox.2,
                by1 = bbox.3,
                ia = face_metrics.italic_angle as i32,
                asc = ascent,
                dsc = descent,
                cap = cap_height,
                file = fontfile_id,
            );
            self.buf.extend_from_slice(desc_body.as_bytes());
            self.end_object();

            // 4) Embedded font program, FlateDecode compressed. TrueType goes
            //    in a FontFile2 stream carrying /Length1 (the uncompressed
            //    sfnt size); CFF/OpenType goes in a FontFile3 stream tagged
            //    /Subtype /OpenType (a complete OpenType program).
            let compressed = deflate(ttf_bytes);
            self.start_object(fontfile_id);
            let header = if is_cff {
                format!(
                    "<< /Length {len} /Subtype /OpenType /Filter /FlateDecode >>\nstream\n",
                    len = compressed.len(),
                )
            } else {
                format!(
                    "<< /Length {len} /Length1 {orig} /Filter /FlateDecode >>\nstream\n",
                    len = compressed.len(),
                    orig = ttf_bytes.len(),
                )
            };
            self.buf.extend_from_slice(header.as_bytes());
            self.buf.extend_from_slice(&compressed);
            self.buf.extend_from_slice(b"\nendstream");
            self.end_object();

            // 5) ToUnicode CMap — tells PDF readers how to reverse-map glyph
            //    IDs back to Unicode codepoints. Without this, copy/paste
            //    and text search in PDF viewers (okular, Acrobat, Preview)
            //    yield gibberish since Identity-H text streams contain
            //    bare glyph IDs rather than character codes.
            // ToUnicode CMap is keyed by CID, and our CIDs are the
            // *original* glyph IDs (the CIDToGIDMap translates them at
            // render time). So the cmap can use the original face's
            // codepoint table directly without any remapping.
            let cmap = build_tounicode_cmap(face_metrics, &fonts.bytes[i], ps_name);
            let cmap_compressed = deflate(cmap.as_bytes());
            self.start_object(tounicode_id);
            let header = format!(
                "<< /Length {len} /Filter /FlateDecode >>\nstream\n",
                len = cmap_compressed.len(),
            );
            self.buf.extend_from_slice(header.as_bytes());
            self.buf.extend_from_slice(&cmap_compressed);
            self.buf.extend_from_slice(b"\nendstream");
            self.end_object();

            // 6) CIDToGIDMap — binary stream of u16-BE entries, one per
            // possible CID. `map[orig_gid] = subset_gid`. PDF reads this
            // to know which glyph in the *subset* font corresponds to the
            // CID it found in the Tj literal. Emitted only on the glyf path;
            // CFF descendants (CIDFontType0) have no CIDToGIDMap.
            if let (Some(cidtogid_id), Some(remapper)) = (cidtogid_id, remapper.as_ref()) {
                let max_orig = used[i].iter().copied().max().unwrap_or(0);
                let map_len = (max_orig as usize + 1) * 2;
                let mut map_bytes = vec![0u8; map_len];
                for &orig_gid in &used[i] {
                    if let Some(new_gid) = remapper.get(orig_gid) {
                        let off = orig_gid as usize * 2;
                        map_bytes[off] = (new_gid >> 8) as u8;
                        map_bytes[off + 1] = (new_gid & 0xFF) as u8;
                    }
                }
                let map_compressed = deflate(&map_bytes);
                self.start_object(cidtogid_id);
                let header = format!(
                    "<< /Length {len} /Filter /FlateDecode >>\nstream\n",
                    len = map_compressed.len(),
                );
                self.buf.extend_from_slice(header.as_bytes());
                self.buf.extend_from_slice(&map_compressed);
                self.buf.extend_from_slice(b"\nendstream");
                self.end_object();
            }
        }
    }

    fn write_images(&mut self, images: &[DecodedImage]) {
        for (i, img) in images.iter().enumerate() {
            self.start_object(self.image_ids[i]);
            let header = format!(
                "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /{} /BitsPerComponent {} /Filter /{} /Length {} >>\nstream\n",
                img.width, img.height, img.colorspace, img.bpc, img.filter, img.data.len(),
            );
            self.buf.extend_from_slice(header.as_bytes());
            self.buf.extend_from_slice(&img.data);
            self.buf.extend_from_slice(b"\nendstream");
            self.end_object();
        }
    }

    fn write_compressed_stream(&mut self, id: u32, data: &[u8]) {
        self.start_object(id);
        let header = format!(
            "<< /Length {} /Filter /FlateDecode >>\nstream\n",
            data.len()
        );
        self.buf.extend_from_slice(header.as_bytes());
        self.buf.extend_from_slice(data);
        self.buf.extend_from_slice(b"\nendstream");
        self.end_object();
    }

    fn write_form_xobject(&mut self, id: u32, content: &[u8], w_pt: u32, h_pt: u32) {
        let compressed = deflate(content);
        self.start_object(id);
        let mut res = String::from("<< /Font << ");
        for i in 0..self.font_ids.len() {
            res.push_str(&format!("/F{} {} 0 R ", i + 1, self.font_ids[i]));
        }
        res.push_str(">>");
        if !self.image_ids.is_empty() {
            res.push_str(" /XObject << ");
            for (i, im_id) in self.image_ids.iter().enumerate() {
                res.push_str(&format!("/Im{} {} 0 R ", i + 1, im_id));
            }
            res.push_str(">>");
        }
        res.push_str(" >>");
        let header = format!(
            "<< /Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 {w} {h}] /Resources {res} /Length {len} /Filter /FlateDecode >>\nstream\n",
            w = w_pt,
            h = h_pt,
            res = res,
            len = compressed.len(),
        );
        self.buf.extend_from_slice(header.as_bytes());
        self.buf.extend_from_slice(&compressed);
        self.buf.extend_from_slice(b"\nendstream\n");
        self.end_object();
    }

    fn write_handout_page(
        &mut self,
        page_id: u32,
        content_id: u32,
        w_pt: u32,
        h_pt: u32,
        form_ids: &[u32],
    ) {
        self.start_object(page_id);
        let mut res = String::from("<< /Font << ");
        for i in 0..self.font_ids.len() {
            res.push_str(&format!("/F{} {} 0 R ", i + 1, self.font_ids[i]));
        }
        res.push_str(">>");
        res.push_str(" /XObject << ");
        for (i, fid) in form_ids.iter().enumerate() {
            res.push_str(&format!("/S{} {} 0 R ", i + 1, fid));
        }
        res.push_str(">> >>");
        let body = format!(
            "<< /Type /Page /Parent {} 0 R /MediaBox [0 0 {} {}] /Resources {} /Contents {} 0 R >>",
            self.pages_id, w_pt, h_pt, res, content_id,
        );
        self.buf.extend_from_slice(body.as_bytes());
        self.end_object();
    }

    fn write_page_with_trans(
        &mut self,
        page_id: u32,
        content_id: u32,
        w_pt: u32,
        h_pt: u32,
        annot_ids: &[u32],
        trans_dict: &str,
    ) {
        self.start_object(page_id);
        let mut res = String::from("<< /Font << ");
        for i in 0..self.font_ids.len() {
            res.push_str(&format!("/F{} {} 0 R ", i + 1, self.font_ids[i]));
        }
        res.push_str(">>");
        if !self.image_ids.is_empty() {
            res.push_str(" /XObject << ");
            for (i, id) in self.image_ids.iter().enumerate() {
                res.push_str(&format!("/Im{} {} 0 R ", i + 1, id));
            }
            res.push_str(">>");
        }
        res.push_str(" >>");
        let annots = if annot_ids.is_empty() {
            String::new()
        } else {
            let mut s = String::from(" /Annots [");
            for id in annot_ids {
                s.push_str(&format!("{} 0 R ", id));
            }
            s.push(']');
            s
        };
        let body = format!(
            "<< /Type /Page /Parent {} 0 R /MediaBox [0 0 {} {}] /Resources {} /Contents {} 0 R{}{} >>",
            self.pages_id, w_pt, h_pt, res, content_id, annots, trans_dict,
        );
        self.buf.extend_from_slice(body.as_bytes());
        self.end_object();
    }

    fn write_link_annot(&mut self, id: u32, rect: &LinkRect) {
        self.start_object(id);
        let uri = pdf_escape_string(&rect.uri);
        let mut body = Vec::new();
        let _ = write!(
            &mut body,
            "<< /Type /Annot /Subtype /Link /Rect [{:.3} {:.3} {:.3} {:.3}] /Border [0 0 0] /A << /Type /Action /S /URI /URI (",
            rect.llx, rect.lly, rect.urx, rect.ury,
        );
        body.extend_from_slice(&uri);
        body.extend_from_slice(b") >> >>");
        self.buf.extend_from_slice(&body);
        self.end_object();
    }

    fn write_pages_tree(&mut self, page_ids: &[u32]) {
        self.start_object(self.pages_id);
        let mut kids = String::from("[");
        for id in page_ids {
            kids.push_str(&format!("{} 0 R ", id));
        }
        kids.push(']');
        let body = format!(
            "<< /Type /Pages /Kids {} /Count {} >>",
            kids,
            page_ids.len(),
        );
        self.buf.extend_from_slice(body.as_bytes());
        self.end_object();
    }

    fn write_catalog(&mut self) {
        self.start_object(self.catalog_id);
        let body = format!("<< /Type /Catalog /Pages {} 0 R >>", self.pages_id);
        self.buf.extend_from_slice(body.as_bytes());
        self.end_object();
    }

    fn write_xref_and_trailer(&mut self) {
        let xref_offset = self.buf.len() as u64;
        let total = self.next_id as usize + 1;
        self.buf
            .extend_from_slice(format!("xref\n0 {}\n", total).as_bytes());
        self.buf.extend_from_slice(b"0000000000 65535 f \n");
        for i in 0..self.next_id as usize {
            self.buf
                .extend_from_slice(format!("{:010} 00000 n \n", self.offsets[i]).as_bytes());
        }
        let trailer = format!(
            "trailer\n<< /Size {} /Root {} 0 R /Info {} 0 R >>\nstartxref\n{}\n%%EOF\n",
            total, self.catalog_id, self.info_id, xref_offset,
        );
        self.buf.extend_from_slice(trailer.as_bytes());
        let _ = self.slide_w;
        let _ = self.slide_h;
    }
}

// ---------------------------------------------------------------------------
// Slide rendering
// ---------------------------------------------------------------------------

struct SlideRenderer<'a> {
    theme: &'a Theme,
    layout: &'a Layout,
    page_w: u32,
    page_h: u32,
    ops: Vec<u8>,
    links: Vec<LinkRect>,
    fonts: &'a PdfFonts,
    /// Per-face set of glyph IDs we've emitted. Drives font subsetting at
    /// write time so the PDF only carries the glyphs it actually uses.
    used_glyphs: Vec<std::collections::HashSet<u16>>,
    /// Slide-level text alignment (`align=`), applied to body paragraphs and
    /// headings. Reset per slide in [`render_content_slide`].
    cur_text_align: TextAlign,
    /// Slide-level left-column fraction (`width=`), applied to Columns.
    /// `None` = even split. Reset per slide.
    cur_col_frac: Option<f32>,
    /// Slide-level vertical alignment (`valign=`): "top"/"center"/"bottom".
    /// Drives per-column centring in the Columns arm. Reset per slide.
    cur_valign: &'static str,
    /// Per-slide body-text scale (`text-scale`); 1.0 = theme default.
    cur_text_scale: f32,
}

struct LinkRect {
    uri: String,
    llx: f32,
    lly: f32,
    urx: f32,
    ury: f32,
}

impl<'a> SlideRenderer<'a> {
    fn new(
        theme: &'a Theme,
        layout: &'a Layout,
        page_w: u32,
        page_h: u32,
        fonts: &'a PdfFonts,
    ) -> Self {
        SlideRenderer {
            theme,
            layout,
            page_w,
            page_h,
            ops: Vec::with_capacity(8 * 1024),
            links: Vec::new(),
            fonts,
            used_glyphs: vec![std::collections::HashSet::new(); fonts.face_count()],
            cur_text_align: TextAlign::Left,
            cur_col_frac: None,
            cur_valign: "top",
            cur_text_scale: 1.0,
        }
    }

    fn finish(self) -> (Vec<u8>, Vec<LinkRect>, Vec<std::collections::HashSet<u16>>) {
        (self.ops, self.links, self.used_glyphs)
    }

    fn pt(&self, emu: u32) -> f32 {
        emu as f32 / EMU_PER_PT
    }

    fn pdf_y(&self, emu_y: u32) -> f32 {
        self.page_h as f32 - self.pt(emu_y)
    }

    /// Fill the entire page with a color (used for section slide backgrounds).
    fn fill_background(&mut self, hex: &str) {
        let (r, g, b) = hex_to_rgb_f(hex);
        let _ = write!(
            &mut self.ops,
            "{:.3} {:.3} {:.3} rg\n0 0 {} {} re f\n",
            r, g, b, self.page_w, self.page_h
        );
    }

    fn rect(&mut self, x: u32, y: u32, w: u32, h: u32, hex: &str) {
        let (r, g, b) = hex_to_rgb_f(hex);
        let xp = self.pt(x);
        let wp = self.pt(w);
        let hp = self.pt(h);
        let yp = self.pdf_y(y) - hp;
        let _ = write!(
            &mut self.ops,
            "{:.3} {:.3} {:.3} rg\n{:.3} {:.3} {:.3} {:.3} re f\n",
            r, g, b, xp, yp, wp, hp,
        );
    }

    /// Render a text string at (x_emu, y_emu) where y is the top of the box.
    /// Auto-wraps to multiple lines if the text is wider than max_w_emu.
    /// Returns the number of wrapped lines actually rendered. Callers
    /// laying out a multi-block region (title slide, section divider)
    /// can use this to offset the next block downward when the text
    /// wraps. Most callers ignore the return value.
    fn text_line(
        &mut self,
        x: u32,
        y: u32,
        text: &str,
        size_centipt: u32,
        color_hex: &str,
        bold: bool,
        italic: bool,
        mono: bool,
        align: TextAlign,
        max_w_emu: u32,
    ) -> usize {
        let (r, g, b) = hex_to_rgb_f(color_hex);
        let font_idx = font_index(bold, italic, mono);
        let size_pt = size_centipt as f32 / 100.0;
        let total_width = text_width_pt(self.fonts, text, font_idx, size_pt);
        let max_w_pt = self.pt(max_w_emu);

        let lines: Vec<String> = if total_width <= max_w_pt || max_w_pt <= 0.0 {
            vec![text.to_string()]
        } else {
            wrap_text_simple(self.fonts, text, font_idx, size_pt, max_w_pt)
        };

        let line_h_pt = size_pt * 1.25;
        let baseline_first = self.pdf_y(y) - size_pt * 0.78;
        for (i, line_text) in lines.iter().enumerate() {
            let line_width = text_width_pt(self.fonts, line_text, font_idx, size_pt);
            let xp = match align {
                TextAlign::Left => self.pt(x),
                TextAlign::Center => self.pt(x) + (max_w_pt - line_width) / 2.0,
                TextAlign::Right => self.pt(x) + max_w_pt - line_width,
            };
            let baseline = baseline_first - i as f32 * line_h_pt;
            // Split the line by face — chars without a glyph in the primary
            // face fall back to the CJK face (if loaded). Each run gets its
            // own BT…ET so PDF only sees one font per Tj.
            let runs = glyph_hex_runs(self.fonts, line_text, font_idx, size_pt);
            let mut cur_x = xp;
            for (face, hex, adv) in runs {
                record_glyphs_from_hex(&mut self.used_glyphs[face], &hex);
                let _ = write!(
                    &mut self.ops,
                    "BT\n/F{} {:.2} Tf\n{:.3} {:.3} {:.3} rg\n{:.3} {:.3} Td\n{} Tj\nET\n",
                    face + 1,
                    size_pt,
                    r,
                    g,
                    b,
                    cur_x,
                    baseline,
                    hex,
                );
                cur_x += adv;
            }
        }
        lines.len()
    }

    fn text_at_baseline_pt(
        &mut self,
        x_pt: f32,
        baseline_pt: f32,
        text: &str,
        size_pt: f32,
        color_hex: &str,
        italic: bool,
        bold: bool,
    ) {
        if text.trim().is_empty() || !x_pt.is_finite() || !baseline_pt.is_finite() {
            return;
        }
        let (r, g, b) = hex_to_rgb_f(color_hex);
        let font_idx = match (italic, bold) {
            (false, false) => FONT_HELV,
            (true, false) => FONT_HELV_OBL,
            (false, true) => FONT_HELV_BOLD,
            (true, true) => FONT_HELV_BOLD_OBL,
        };
        let runs = glyph_hex_runs(self.fonts, text, font_idx, size_pt.max(1.0));
        let mut cur_x = x_pt;
        for (face, hex, adv) in runs {
            record_glyphs_from_hex(&mut self.used_glyphs[face], &hex);
            let _ = write!(
                &mut self.ops,
                "BT\n/F{} {:.2} Tf\n{:.3} {:.3} {:.3} rg\n{:.3} {:.3} Td\n{} Tj\nET\n",
                face + 1,
                size_pt.max(1.0),
                r,
                g,
                b,
                cur_x,
                baseline_pt,
                hex,
            );
            cur_x += adv;
        }
    }

    fn draw_math_text_layout(
        &mut self,
        layout: &crate::math::MathTextLayout,
        origin_x_pt: f32,
        top_y_pt: f32,
        scale: f32,
        color_hex: &str,
    ) {
        for draw in &layout.draws {
            match draw {
                crate::math::MathLayoutDraw::Text {
                    x,
                    y,
                    size,
                    text,
                    bold,
                } => {
                    let (x_pt, baseline_pt) =
                        self.math_local_to_pdf_pt(origin_x_pt, top_y_pt, scale, *x, *y);
                    self.text_at_baseline_pt(
                        x_pt,
                        baseline_pt,
                        text,
                        size * scale,
                        color_hex,
                        true,
                        *bold,
                    );
                }
                crate::math::MathLayoutDraw::Line {
                    x1,
                    y1,
                    x2,
                    y2,
                    stroke_width,
                } => {
                    let p1 = self.math_local_to_pdf_pt(origin_x_pt, top_y_pt, scale, *x1, *y1);
                    let p2 = self.math_local_to_pdf_pt(origin_x_pt, top_y_pt, scale, *x2, *y2);
                    self.stroke_line_pt(p1, p2, (stroke_width * scale).max(0.22), color_hex);
                }
                crate::math::MathLayoutDraw::Polyline {
                    points,
                    stroke_width,
                } => {
                    let points = points
                        .iter()
                        .map(|(x, y)| {
                            self.math_local_to_pdf_pt(origin_x_pt, top_y_pt, scale, *x, *y)
                        })
                        .collect::<Vec<_>>();
                    self.stroke_polyline_pt(&points, (stroke_width * scale).max(0.22), color_hex);
                }
                crate::math::MathLayoutDraw::Delimiter {
                    x,
                    y,
                    width,
                    height,
                    token,
                    stroke_width,
                } => self.draw_math_delimiter_pt(
                    origin_x_pt,
                    top_y_pt,
                    scale,
                    (*x, *y, *width, *height),
                    token,
                    (stroke_width * scale).max(0.22),
                    color_hex,
                ),
            }
        }
    }

    fn math_local_to_pdf_pt(
        &self,
        origin_x_pt: f32,
        top_y_pt: f32,
        scale: f32,
        x: f32,
        y: f32,
    ) -> (f32, f32) {
        (
            origin_x_pt + x * scale,
            self.page_h as f32 - (top_y_pt + y * scale),
        )
    }

    fn stroke_line_pt(
        &mut self,
        p1: (f32, f32),
        p2: (f32, f32),
        stroke_width_pt: f32,
        color_hex: &str,
    ) {
        self.stroke_path_pt(
            &format!("{:.3} {:.3} m\n{:.3} {:.3} l\n", p1.0, p1.1, p2.0, p2.1),
            stroke_width_pt,
            color_hex,
        );
    }

    fn stroke_polyline_pt(&mut self, points: &[(f32, f32)], stroke_width_pt: f32, color_hex: &str) {
        if points.len() < 2 {
            return;
        }
        let mut path = format!("{:.3} {:.3} m\n", points[0].0, points[0].1);
        for (x, y) in &points[1..] {
            path.push_str(&format!("{x:.3} {y:.3} l\n"));
        }
        self.stroke_path_pt(&path, stroke_width_pt, color_hex);
    }

    fn stroke_path_pt(&mut self, path: &str, stroke_width_pt: f32, color_hex: &str) {
        if !stroke_width_pt.is_finite() || stroke_width_pt <= 0.0 {
            return;
        }
        let (r, g, b) = hex_to_rgb_f(color_hex);
        let _ = write!(
            &mut self.ops,
            "q\n{:.3} {:.3} {:.3} RG\n{:.3} w\n1 J\n1 j\n{}S\nQ\n",
            r, g, b, stroke_width_pt, path,
        );
    }

    fn draw_math_delimiter_pt(
        &mut self,
        origin_x_pt: f32,
        top_y_pt: f32,
        scale: f32,
        rect: (f32, f32, f32, f32),
        token: &str,
        stroke_width_pt: f32,
        color_hex: &str,
    ) {
        let (x, y, width, height) = rect;
        let page_h = self.page_h as f32;
        let pt = |lx: f32, ly: f32| -> (f32, f32) {
            (origin_x_pt + lx * scale, page_h - (top_y_pt + ly * scale))
        };
        match token {
            "(" => {
                let p0 = pt(x + width * 0.86, y);
                let c1 = pt(x + width * 0.10, y + height * 0.20);
                let c2 = pt(x + width * 0.10, y + height * 0.80);
                let p1 = pt(x + width * 0.86, y + height);
                let path = format!(
                    "{:.3} {:.3} m\n{:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c\n",
                    p0.0, p0.1, c1.0, c1.1, c2.0, c2.1, p1.0, p1.1
                );
                self.stroke_path_pt(&path, stroke_width_pt, color_hex);
            }
            ")" => {
                let p0 = pt(x + width * 0.14, y);
                let c1 = pt(x + width * 0.90, y + height * 0.20);
                let c2 = pt(x + width * 0.90, y + height * 0.80);
                let p1 = pt(x + width * 0.14, y + height);
                let path = format!(
                    "{:.3} {:.3} m\n{:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c\n",
                    p0.0, p0.1, c1.0, c1.1, c2.0, c2.1, p1.0, p1.1
                );
                self.stroke_path_pt(&path, stroke_width_pt, color_hex);
            }
            "[" => self.stroke_polyline_pt(
                &[
                    pt(x + width, y),
                    pt(x, y),
                    pt(x, y + height),
                    pt(x + width, y + height),
                ],
                stroke_width_pt,
                color_hex,
            ),
            "]" => self.stroke_polyline_pt(
                &[
                    pt(x, y),
                    pt(x + width, y),
                    pt(x + width, y + height),
                    pt(x, y + height),
                ],
                stroke_width_pt,
                color_hex,
            ),
            "{" => {
                let p0 = pt(x + width, y);
                let c1 = pt(x + width * 0.18, y);
                let c2 = pt(x + width * 0.20, y + height * 0.28);
                let p1 = pt(x + width * 0.56, y + height * 0.38);
                let c3 = pt(x + width * 0.82, y + height * 0.46);
                let c4 = pt(x + width * 0.18, y + height * 0.45);
                let p2 = pt(x + width * 0.18, y + height * 0.50);
                let c5 = pt(x + width * 0.18, y + height * 0.55);
                let c6 = pt(x + width * 0.82, y + height * 0.54);
                let p3 = pt(x + width * 0.56, y + height * 0.62);
                let c7 = pt(x + width * 0.20, y + height * 0.72);
                let c8 = pt(x + width * 0.18, y + height);
                let p4 = pt(x + width, y + height);
                let path = format!(
                    "{:.3} {:.3} m\n{:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c\n{:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c\n{:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c\n{:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c\n",
                    p0.0, p0.1,
                    c1.0, c1.1, c2.0, c2.1, p1.0, p1.1,
                    c3.0, c3.1, c4.0, c4.1, p2.0, p2.1,
                    c5.0, c5.1, c6.0, c6.1, p3.0, p3.1,
                    c7.0, c7.1, c8.0, c8.1, p4.0, p4.1
                );
                self.stroke_path_pt(&path, stroke_width_pt, color_hex);
            }
            "}" => {
                let p0 = pt(x, y);
                let c1 = pt(x + width * 0.82, y);
                let c2 = pt(x + width * 0.80, y + height * 0.28);
                let p1 = pt(x + width * 0.44, y + height * 0.38);
                let c3 = pt(x + width * 0.18, y + height * 0.46);
                let c4 = pt(x + width * 0.82, y + height * 0.45);
                let p2 = pt(x + width * 0.82, y + height * 0.50);
                let c5 = pt(x + width * 0.82, y + height * 0.55);
                let c6 = pt(x + width * 0.18, y + height * 0.54);
                let p3 = pt(x + width * 0.44, y + height * 0.62);
                let c7 = pt(x + width * 0.80, y + height * 0.72);
                let c8 = pt(x + width * 0.82, y + height);
                let p4 = pt(x, y + height);
                let path = format!(
                    "{:.3} {:.3} m\n{:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c\n{:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c\n{:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c\n{:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c\n",
                    p0.0, p0.1,
                    c1.0, c1.1, c2.0, c2.1, p1.0, p1.1,
                    c3.0, c3.1, c4.0, c4.1, p2.0, p2.1,
                    c5.0, c5.1, c6.0, c6.1, p3.0, p3.1,
                    c7.0, c7.1, c8.0, c8.1, p4.0, p4.1
                );
                self.stroke_path_pt(&path, stroke_width_pt, color_hex);
            }
            "|" => self.stroke_line_pt(
                pt(x + width * 0.5, y),
                pt(x + width * 0.5, y + height),
                stroke_width_pt,
                color_hex,
            ),
            "‖" => {
                self.stroke_line_pt(
                    pt(x + width * 0.35, y),
                    pt(x + width * 0.35, y + height),
                    stroke_width_pt,
                    color_hex,
                );
                self.stroke_line_pt(
                    pt(x + width * 0.65, y),
                    pt(x + width * 0.65, y + height),
                    stroke_width_pt,
                    color_hex,
                );
            }
            "⟨" => self.stroke_polyline_pt(
                &[
                    pt(x + width, y),
                    pt(x, y + height * 0.5),
                    pt(x + width, y + height),
                ],
                stroke_width_pt,
                color_hex,
            ),
            "⟩" => self.stroke_polyline_pt(
                &[pt(x, y), pt(x + width, y + height * 0.5), pt(x, y + height)],
                stroke_width_pt,
                color_hex,
            ),
            _ => {
                let (x_pt, baseline_pt) =
                    self.math_local_to_pdf_pt(origin_x_pt, top_y_pt, scale, x, y + height * 0.82);
                self.text_at_baseline_pt(
                    x_pt,
                    baseline_pt,
                    token,
                    height * scale,
                    color_hex,
                    true,
                    false,
                );
            }
        }
    }

    /// EMU equivalent of one line at the given font size, matching the
    /// 1.25 leading used inside [`text_line`]. Lets title-slide layouts
    /// push the subtitle / author down by exactly the right amount when
    /// the title wraps.
    fn line_h_emu(size_centipt: u32) -> u32 {
        let pt = size_centipt as f32 / 100.0;
        (pt * 1.25 * EMU_PER_PT) as u32
    }

    /// Largest title size (≤ `base`, in centipt) at which `text` wraps within
    /// `max_w_emu` and the wrapped block fits inside `max_h_emu`. Stops long
    /// hero titles / section dividers from overflowing the slide and footer.
    fn fit_hero_size(&self, text: &str, base: u32, max_w_emu: u32, max_h_emu: u32) -> u32 {
        let font_idx = font_index(true, false, false);
        let max_w_pt = self.pt(max_w_emu);
        let mut size = base;
        for _ in 0..12 {
            let pt = size as f32 / 100.0;
            let lines =
                if max_w_pt <= 0.0 || text_width_pt(self.fonts, text, font_idx, pt) <= max_w_pt {
                    1
                } else {
                    wrap_text_simple(self.fonts, text, font_idx, pt, max_w_pt)
                        .len()
                        .max(1)
                };
            let block_h = lines as u32 * Self::line_h_emu(size);
            if block_h <= max_h_emu || size <= 1400 {
                break;
            }
            // Shrink toward the height target; sqrt because a smaller font both
            // shortens each line and fits more words per line.
            let ratio = (max_h_emu as f32 / block_h as f32).sqrt();
            let next = ((size as f32 * ratio).floor() as u32).max(1400);
            size = if next >= size { size - 100 } else { next };
        }
        size
    }

    /// Wrap a vector of runs into lines for the given width and emit them.
    /// Returns the y position after the last line.
    fn paragraph(
        &mut self,
        runs: &[Run],
        x: u32,
        y_start: u32,
        max_w_emu: u32,
        size_centipt: u32,
        base_color_hex: &str,
        base_bold: bool,
        base_italic: bool,
    ) -> u32 {
        let lines = wrap_runs(
            self.fonts,
            runs,
            self.pt(max_w_emu),
            size_centipt as f32 / 100.0,
            base_bold,
            base_italic,
        );
        let line_height_emu = (size_centipt as f32 * 1.25 * EMU_PER_PT / 100.0) as u32;
        let size_pt = size_centipt as f32 / 100.0;
        let mut y = y_start;
        for line in lines {
            // Slide-level alignment: shift the line within [x, x+max_w] by its
            // measured width. Left alignment is the no-op fast path.
            let lx = if matches!(self.cur_text_align, TextAlign::Left) {
                x
            } else {
                let line_w_pt: f32 = line
                    .iter()
                    .map(|r| {
                        let fi = font_index(base_bold || r.bold, base_italic || r.italic, r.code);
                        text_width_pt(self.fonts, &r.text, fi, size_pt)
                    })
                    .sum();
                let line_w_emu = (line_w_pt * EMU_PER_PT) as u32;
                let slack = max_w_emu.saturating_sub(line_w_emu);
                match self.cur_text_align {
                    TextAlign::Center => x + slack / 2,
                    TextAlign::Right => x + slack,
                    TextAlign::Left => x,
                }
            };
            self.text_runs_line(
                &line,
                lx,
                y,
                size_centipt,
                base_color_hex,
                base_bold,
                base_italic,
            );
            y += line_height_emu;
        }
        y
    }

    fn text_runs_line(
        &mut self,
        runs: &[Run],
        x: u32,
        y: u32,
        size_centipt: u32,
        base_color_hex: &str,
        base_bold: bool,
        base_italic: bool,
    ) {
        let size_pt = size_centipt as f32 / 100.0;
        let mut cursor_pt = self.pt(x);
        let baseline = self.pdf_y(y) - size_pt * 0.78;
        for r in runs {
            if r.text.is_empty() {
                continue;
            }
            let bold = base_bold || r.bold;
            let italic = base_italic || r.italic;
            let mono = r.code;
            let font_idx = font_index(bold, italic, mono);
            let color_hex = if r.link.is_some() {
                &self.theme.link
            } else if r.code {
                &self.theme.code_accent
            } else {
                base_color_hex
            };
            let (rc, gc, bc) = hex_to_rgb_f(color_hex);
            let width = text_width_pt(self.fonts, &r.text, font_idx, size_pt);
            let runs = glyph_hex_runs(self.fonts, &r.text, font_idx, size_pt);
            let mut cur_x = cursor_pt;
            for (face, hex, adv) in runs {
                record_glyphs_from_hex(&mut self.used_glyphs[face], &hex);
                let _ = write!(
                    &mut self.ops,
                    "BT\n/F{} {:.2} Tf\n{:.3} {:.3} {:.3} rg\n{:.3} {:.3} Td\n{} Tj\nET\n",
                    face + 1,
                    size_pt,
                    rc,
                    gc,
                    bc,
                    cur_x,
                    baseline,
                    hex,
                );
                cur_x += adv;
            }
            if let Some(uri) = r.link.as_deref() {
                let underline_y = baseline - size_pt * 0.08;
                let (lr, lg, lb) = hex_to_rgb_f(color_hex);
                let _ = write!(
                    &mut self.ops,
                    "{:.3} {:.3} {:.3} rg\n{:.3} {:.3} {:.3} {:.3} re f\n",
                    lr,
                    lg,
                    lb,
                    cursor_pt,
                    underline_y,
                    width,
                    size_pt * 0.05,
                );
                self.links.push(LinkRect {
                    uri: uri.to_string(),
                    llx: cursor_pt,
                    lly: underline_y,
                    urx: cursor_pt + width,
                    ury: baseline + size_pt * 0.85,
                });
            }
            if r.strike {
                let strike_y = baseline + size_pt * 0.25;
                let (sr, sg, sb) = hex_to_rgb_f(color_hex);
                let _ = write!(
                    &mut self.ops,
                    "{:.3} {:.3} {:.3} rg\n{:.3} {:.3} {:.3} {:.3} re f\n",
                    sr,
                    sg,
                    sb,
                    cursor_pt,
                    strike_y,
                    width,
                    size_pt * 0.05,
                );
            }
            cursor_pt += width;
        }
    }

    fn draw_image(&mut self, src: &str, x: u32, y: u32, w: u32, h: u32, imgs: &Imgs) {
        let Some(idx) = imgs.index(src) else {
            return;
        };
        let xp = self.pt(x);
        let wp = self.pt(w);
        let hp = self.pt(h);
        let yp = self.pdf_y(y) - hp;
        let _ = write!(
            &mut self.ops,
            "q\n{:.3} 0 0 {:.3} {:.3} {:.3} cm\n/Im{} Do\nQ\n",
            wp,
            hp,
            xp,
            yp,
            idx + 1,
        );
    }

    // -------------------- entry point per slide --------------------

    fn render(&mut self, slide: &Slide, num: usize, total: usize, deck_title: &str, imgs: &Imgs) {
        let bg = slide
            .bg_color()
            .map(|c| resolve_bg_color(c, self.theme))
            .unwrap_or_else(|| self.theme.bg.clone());
        self.fill_background(&bg);

        if let Some(bg) = &slide.bg_image {
            if imgs.index(bg).is_some() {
                self.draw_image(bg, 0, 0, self.theme.slide_w, self.theme.slide_h, imgs);
            }
        }

        if self.render_full_page_image_slide(slide, imgs) {
            return;
        }
        if self.render_full_page_code_slide(slide) {
            return;
        }

        match &slide.kind {
            SlideKind::Title {
                subtitle,
                author,
                date,
            } => {
                self.render_title_slide(
                    slide,
                    subtitle.as_deref(),
                    author.as_deref(),
                    date.as_deref(),
                );
            }
            SlideKind::Section => {
                self.render_section_slide(slide, num, total);
            }
            SlideKind::Content => {
                self.render_content_slide(slide, num, total, deck_title, imgs);
            }
        }
    }

    fn render_full_page_image_slide(&mut self, slide: &Slide, imgs: &Imgs) -> bool {
        let Some((src, _, _)) = slide.full_page_image() else {
            return false;
        };

        let (display_w, display_h) = if let Some((iw, ih)) = imgs.dims(src) {
            fit_image(iw, ih, self.theme.slide_w, self.theme.slide_h)
        } else {
            (self.theme.slide_w, self.theme.slide_h)
        };
        let x = self.theme.slide_w.saturating_sub(display_w) / 2;
        let y = self.theme.slide_h.saturating_sub(display_h) / 2;
        self.draw_image(src, x, y, display_w, display_h, imgs);
        true
    }

    fn render_full_page_code_slide(&mut self, slide: &Slide) -> bool {
        let Some((lines, lang)) = slide.full_page_code() else {
            return false;
        };

        let math_markup = crate::math::is_markup_text_language(lang);
        if math_markup {
            self.render_full_page_math_markup(lines);
            return true;
        }

        let rendered_lines = crate::math::translate_markup_lines(lines, lang);
        let margin_x = if self.theme.portrait { 280000 } else { 360000 };
        let margin_y = if self.theme.portrait { 320000 } else { 300000 };
        let max_w = self.theme.slide_w.saturating_sub(margin_x * 2);
        let max_h = self.theme.slide_h.saturating_sub(margin_y * 2);
        let base_size = self
            .theme
            .code_size
            .min(if self.theme.portrait { 850 } else { 950 });
        let base_pt = base_size as f32 / 100.0;
        let font_idx = if math_markup {
            FONT_HELV_OBL
        } else {
            FONT_COUR
        };
        let max_line_pt = rendered_lines
            .iter()
            .map(|line| text_width_pt(self.fonts, line, font_idx, base_pt))
            .fold(1.0_f32, f32::max);
        let max_w_pt = self.pt(max_w).max(1.0);
        let line_count = rendered_lines.len().max(1) as f32;
        let max_h_pt = self.pt(max_h).max(1.0);
        let line_h_factor = if math_markup { 1.10_f32 } else { 1.18_f32 };
        let scale_w = max_w_pt / max_line_pt;
        let scale_h = max_h_pt / (line_count * base_pt * line_h_factor);
        let scale = scale_w.min(scale_h).min(1.0);
        let size = ((base_size as f32) * scale).clamp(450.0, base_size as f32) as u32;
        let line_h = (size as f32 * line_h_factor * EMU_PER_PT / 100.0) as u32;
        let total_h = line_h.saturating_mul(rendered_lines.len().max(1) as u32);
        let mut y = margin_y + max_h.saturating_sub(total_h) / 2;
        let color = self.theme.title_color.clone();
        for line in &rendered_lines {
            self.text_line(
                margin_x,
                y,
                line,
                size,
                &color,
                false,
                math_markup,
                !math_markup,
                if math_markup {
                    TextAlign::Center
                } else {
                    TextAlign::Left
                },
                if math_markup { max_w } else { 0 },
            );
            y = y.saturating_add(line_h);
        }
        true
    }

    fn render_full_page_math_markup(&mut self, lines: &[String]) {
        let margin_x = if self.theme.portrait { 230000 } else { 320000 };
        let margin_y = if self.theme.portrait { 260000 } else { 260000 };
        let max_w = self.theme.slide_w.saturating_sub(margin_x * 2);
        let max_h = self.theme.slide_h.saturating_sub(margin_y * 2);
        let max_w_pt = self.pt(max_w).max(1.0);
        let max_h_pt = self.pt(max_h).max(1.0);
        let margin_x_pt = self.pt(margin_x);
        let margin_y_pt = self.pt(margin_y);
        let base_size = 28.0_f32;
        let gap = base_size * 0.24;

        // Measure against the face math is actually drawn in (FONT_HELV, the
        // sans regular slot — which `--font` replaces) so reserved widths stay
        // aligned with the rendered glyphs.
        let metrics = PdfMathMetrics {
            fonts: self.fonts,
            font_idx: FONT_HELV,
        };
        let raw_layouts = math_markup_line_layouts(lines, &metrics);
        let layouts = fit_packed_math_markup_line_layouts(
            lines,
            raw_layouts,
            base_size,
            gap,
            max_w_pt / max_h_pt,
            &metrics,
        );
        let (max_line_w, total_h) = math_markup_metrics(&layouts, base_size, gap);
        let scale_w = max_w_pt / max_line_w;
        let scale_h = max_h_pt / total_h.max(1.0);
        let scale = scale_w.min(scale_h).min(1.0).max(0.045);
        let rendered_h = total_h * scale;
        let mut top_y_pt = margin_y_pt + (max_h_pt - rendered_h).max(0.0) / 2.0;
        let color = self.theme.title_color.clone();

        for layout in &layouts {
            if let Some(layout) = layout {
                let line_w_pt = layout.width * scale;
                let x_pt = margin_x_pt + (max_w_pt - line_w_pt).max(0.0) / 2.0;
                self.draw_math_text_layout(layout, x_pt, top_y_pt, scale, &color);
                top_y_pt += layout.height * scale + gap * scale;
            } else {
                top_y_pt += base_size * 0.65 * scale + gap * scale;
            }
        }
    }

    fn render_title_slide(
        &mut self,
        slide: &Slide,
        subtitle: Option<&str>,
        author: Option<&str>,
        date: Option<&str>,
    ) {
        let theme = self.theme;
        let layout = self.layout;
        let w = theme.slide_w;
        let h = theme.slide_h;

        match layout.kind {
            LayoutKind::Clean => {
                self.rect(0, 0, w, 360000, &theme.accent.clone());
                self.rect(
                    600000,
                    h / 2 - 1700000,
                    100000,
                    600000,
                    &theme.accent.clone(),
                );
                // Shrink the hero size if needed so a long title fits between
                // its top and the footer/subtitle region instead of running
                // off the bottom and overprinting the author/date.
                let title_top = h / 2 - 1600000;
                let title_max_h = (h.saturating_sub(1_400_000)).saturating_sub(title_top);
                let hero =
                    self.fit_hero_size(&slide.title, theme.hero_size, w - 1200000, title_max_h);
                let title_lines = self.text_line(
                    800000,
                    title_top,
                    &slide.title,
                    hero,
                    &theme.title_color.clone(),
                    true,
                    false,
                    false,
                    TextAlign::Left,
                    w - 1200000,
                );
                // Push subtitle down by the number of extra title lines so
                // a wrapped 2-line title doesn't overlap the subtitle.
                let extra = (title_lines.saturating_sub(1) as u32) * Self::line_h_emu(hero);
                if let Some(sub) = subtitle {
                    let sub_size = if theme.portrait { 1800 } else { 2400 };
                    self.text_line(
                        800000,
                        h / 2 - 400000 + extra,
                        sub,
                        sub_size,
                        &theme.accent.clone(),
                        false,
                        false,
                        false,
                        TextAlign::Left,
                        w - 1200000,
                    );
                }
                if let Some(text) = author_date(author, date) {
                    self.text_line(
                        800000,
                        h - 700000,
                        &text,
                        1400,
                        &theme.muted_color.clone(),
                        false,
                        false,
                        false,
                        TextAlign::Left,
                        w - 1200000,
                    );
                }
            }
            LayoutKind::Studio => {
                self.rect(0, 0, 90000, h, &theme.accent.clone());
                if let Some(text) = author_date(author, date) {
                    let kicker = letterspaced(&text);
                    self.text_line(
                        900000,
                        900000,
                        &kicker,
                        1200,
                        &theme.muted_color.clone(),
                        true,
                        false,
                        false,
                        TextAlign::Left,
                        w - 1500000,
                    );
                }
                let studio_max_h = (h * 80 / 100).saturating_sub(h / 2 - 1000000);
                let hero =
                    self.fit_hero_size(&slide.title, theme.hero_size, w - 1500000, studio_max_h);
                let title_lines = self.text_line(
                    900000,
                    h / 2 - 1000000,
                    &slide.title,
                    hero,
                    &theme.title_color.clone(),
                    false,
                    true,
                    false,
                    TextAlign::Left,
                    w - 1500000,
                );
                let extra = (title_lines.saturating_sub(1) as u32) * Self::line_h_emu(hero);
                if let Some(sub) = subtitle {
                    let sub_size = if theme.portrait { 1700 } else { 2200 };
                    self.text_line(
                        900000,
                        h / 2 + 500000 + extra,
                        sub,
                        sub_size,
                        &theme.body_color.clone(),
                        false,
                        false,
                        false,
                        TextAlign::Left,
                        w - 1500000,
                    );
                    self.rect(
                        900000,
                        h / 2 + 1300000 + extra,
                        600000,
                        30000,
                        &theme.accent.clone(),
                    );
                }
            }
            LayoutKind::Frame => {
                let sidebar = 2_200_000_u32;
                self.rect(0, 0, sidebar, h, &theme.accent.clone());
                self.rect(300000, 300000, 380000, 40000, &theme.on_accent.clone());
                if let Some(a) = author {
                    self.text_line(
                        300000,
                        h - 1200000,
                        a,
                        1300,
                        &theme.on_accent.clone(),
                        false,
                        false,
                        false,
                        TextAlign::Left,
                        sidebar - 600000,
                    );
                }
                if let Some(d) = date {
                    self.text_line(
                        300000,
                        h - 850000,
                        d,
                        1300,
                        &theme.on_accent.clone(),
                        false,
                        false,
                        false,
                        TextAlign::Left,
                        sidebar - 600000,
                    );
                }
                let title_x = sidebar + 600000;
                let title_w = w - title_x - 600000;
                let frame_max_h = (h * 82 / 100).saturating_sub(h / 2 - 1100000);
                let hero = self.fit_hero_size(&slide.title, theme.hero_size, title_w, frame_max_h);
                let title_lines = self.text_line(
                    title_x,
                    h / 2 - 1100000,
                    &slide.title,
                    hero,
                    &theme.title_color.clone(),
                    true,
                    false,
                    false,
                    TextAlign::Left,
                    title_w,
                );
                let extra = (title_lines.saturating_sub(1) as u32) * Self::line_h_emu(hero);
                if let Some(sub) = subtitle {
                    let sub_size = if theme.portrait { 1700 } else { 2200 };
                    self.text_line(
                        title_x,
                        h / 2 + 400000 + extra,
                        sub,
                        sub_size,
                        &theme.accent.clone(),
                        false,
                        false,
                        false,
                        TextAlign::Left,
                        title_w,
                    );
                }
            }
            LayoutKind::Bold => {
                let block_h = h * 60 / 100;
                self.rect(0, 0, w, block_h, &theme.accent.clone());
                let pad = 700000;
                // Bold's title sits inside the accent block, so a wrapped
                // title naturally grows upward into the block. Pre-compute
                // the line count so we can place the title's baseline to
                // keep its first line inside the block regardless.
                // Shrink the title so it can't grow up and out of the top of
                // the accent block (which would drop it off the slide).
                let hero = self.fit_hero_size(
                    &slide.title,
                    theme.hero_size,
                    w - 2 * pad,
                    block_h.saturating_sub(pad + 1_400_000),
                );
                let hero_pt = hero as f32 / 100.0;
                let title_lines = {
                    let font_idx = font_index(true, false, false);
                    let max_w_pt = self.pt(w - 2 * pad);
                    let total_w = text_width_pt(self.fonts, &slide.title, font_idx, hero_pt);
                    if total_w <= max_w_pt || max_w_pt <= 0.0 {
                        1
                    } else {
                        wrap_text_simple(self.fonts, &slide.title, font_idx, hero_pt, max_w_pt)
                            .len()
                    }
                };
                let title_extra = (title_lines.saturating_sub(1) as u32) * Self::line_h_emu(hero);
                self.text_line(
                    pad,
                    block_h - 1400000 - pad - title_extra,
                    &slide.title,
                    hero,
                    &theme.on_accent.clone(),
                    true,
                    false,
                    false,
                    TextAlign::Left,
                    w - 2 * pad,
                );
                if let Some(sub) = subtitle {
                    let sub_size = if theme.portrait { 1800 } else { 2400 };
                    self.text_line(
                        pad,
                        block_h + 500000,
                        sub,
                        sub_size,
                        &theme.title_color.clone(),
                        false,
                        false,
                        false,
                        TextAlign::Left,
                        w - 2 * pad,
                    );
                }
                if let Some(text) = author_date(author, date) {
                    self.text_line(
                        pad,
                        h - 700000,
                        &text,
                        1400,
                        &theme.muted_color.clone(),
                        false,
                        false,
                        false,
                        TextAlign::Left,
                        w - 2 * pad,
                    );
                }
            }
        }
    }

    fn render_section_slide(&mut self, slide: &Slide, num: usize, total: usize) {
        let theme = self.theme;
        let layout = self.layout;
        let w = theme.slide_w;
        let h = theme.slide_h;

        match layout.kind {
            LayoutKind::Clean | LayoutKind::Frame => {
                if slide.bg_image.is_none() {
                    self.fill_background(&theme.section_bg.clone());
                }
                let bar_w = 1200000;
                self.rect(
                    (w - bar_w) / 2,
                    h / 2 - 1000000,
                    bar_w,
                    60000,
                    &theme.section_text.clone(),
                );
                // Shrink to fit so a long section title doesn't overflow the
                // bottom edge (where it would be silently clipped/lost).
                let sec_top = h / 2 - 200000;
                let sec_max_h = (h.saturating_sub(500_000)).saturating_sub(sec_top);
                let sec_size =
                    self.fit_hero_size(&slide.title, theme.hero_size, w - 1600000, sec_max_h);
                self.text_line(
                    800000,
                    sec_top,
                    &slide.title,
                    sec_size,
                    &theme.section_text.clone(),
                    true,
                    false,
                    false,
                    TextAlign::Center,
                    w - 1600000,
                );
            }
            LayoutKind::Studio => {
                self.rect(0, 0, 90000, h, &theme.accent.clone());
                let huge_size = if theme.portrait { 9000 } else { 13000 };
                let huge = format!("{:02}", num.min(99));
                self.text_line(
                    w - 2_400_000,
                    480000,
                    &huge,
                    huge_size,
                    &theme.divider.clone(),
                    true,
                    false,
                    false,
                    TextAlign::Right,
                    2_300_000,
                );
                let kicker_x = 900000;
                // saturating_sub guards against custom aspects shorter
                // than 2.7 in (e.g. business-card sized decks) where a
                // plain u32 subtraction would underflow and panic.
                let kicker_y = h.saturating_sub(2_600_000);
                let kicker_text = format!(
                    "{}   ·   {}",
                    letterspaced("section"),
                    letterspaced(&format!("{} of {}", num, total))
                );
                self.text_line(
                    kicker_x,
                    kicker_y,
                    &kicker_text,
                    1100,
                    &theme.muted_color.clone(),
                    true,
                    false,
                    false,
                    TextAlign::Left,
                    w - 1800000,
                );
                self.text_line(
                    kicker_x,
                    kicker_y + 700000,
                    &slide.title,
                    theme.hero_size,
                    &theme.title_color.clone(),
                    false,
                    true,
                    false,
                    TextAlign::Left,
                    w - kicker_x - 600000,
                );
                self.rect(
                    kicker_x,
                    kicker_y + 1900000,
                    600000,
                    30000,
                    &theme.accent.clone(),
                );
            }
            LayoutKind::Bold => {
                let block_h = h * 70 / 100;
                self.rect(0, 0, w, block_h, &theme.accent.clone());
                let pad = 700000;
                self.text_line(
                    pad,
                    block_h - 1500000 - pad,
                    &slide.title,
                    theme.hero_size,
                    &theme.on_accent.clone(),
                    true,
                    false,
                    false,
                    TextAlign::Left,
                    w - 2 * pad,
                );
            }
        }
    }

    fn render_content_slide(
        &mut self,
        slide: &Slide,
        num: usize,
        total: usize,
        deck_title: &str,
        imgs: &Imgs,
    ) {
        let theme = self.theme;
        let layout = self.layout;
        // Slide-level formatting hints, threaded into block rendering below.
        self.cur_text_align = match slide.text_align() {
            "center" => TextAlign::Center,
            "right" => TextAlign::Right,
            _ => TextAlign::Left,
        };
        self.cur_col_frac = slide.col_frac();
        self.cur_valign = match slide.valign() {
            "center" => "center",
            "bottom" => "bottom",
            _ => "top",
        };
        self.cur_text_scale = slide.text_scale();
        let w = theme.slide_w;
        let h = theme.slide_h;
        let base_margin: u32 = 533400;
        let left_offset = layout.content_left_offset();
        let extra_left = if layout.shows_rail() { 200000 } else { 0 };
        let content_x = if left_offset > 0 {
            left_offset + 480000
        } else {
            base_margin + extra_left
        };
        let content_w = w.saturating_sub(content_x + base_margin);

        if layout.shows_rail() {
            self.rect(0, 0, layout.rail_width(), h, &theme.accent.clone());
        }
        if layout.shows_sidebar() {
            let sb_w = layout.sidebar_width();
            self.rect(0, 0, sb_w, h, &theme.accent.clone());
            let pad = 300000;
            self.text_line(
                pad,
                pad,
                deck_title,
                1200,
                &theme.on_accent.clone(),
                true,
                false,
                false,
                TextAlign::Left,
                sb_w - 2 * pad,
            );
            self.text_line(
                pad,
                h - 600000,
                &format!("{:02} / {:02}", num, total),
                1100,
                &theme.on_accent.clone(),
                false,
                false,
                false,
                TextAlign::Left,
                sb_w - 2 * pad,
            );
        }

        let title_y: u32 = if matches!(layout.kind, LayoutKind::Bold) {
            280000
        } else {
            360000
        };
        let title_h: u32 = if matches!(layout.kind, LayoutKind::Bold) {
            820000
        } else {
            720000
        };

        // Measure how many lines the title wraps to so a long heading expands
        // the title band instead of overprinting the underline rule and body.
        let title_max_w = if matches!(layout.kind, LayoutKind::Bold) {
            w - 2 * base_margin
        } else {
            content_w
        };
        let title_lines = {
            let title_pt = theme.title_size as f32 / 100.0;
            let font_idx = font_index(true, false, false);
            let max_w_pt = self.pt(title_max_w);
            let total_w = text_width_pt(self.fonts, &slide.title, font_idx, title_pt);
            if total_w <= max_w_pt || max_w_pt <= 0.0 {
                1
            } else {
                wrap_text_simple(self.fonts, &slide.title, font_idx, title_pt, max_w_pt).len()
            }
        };
        let title_extra =
            (title_lines.saturating_sub(1) as u32) * Self::line_h_emu(theme.title_size);

        if matches!(layout.kind, LayoutKind::Bold) {
            let block_h = title_h + 240000 + title_extra;
            self.rect(0, 0, w, title_y + block_h, &theme.accent.clone());
            self.text_line(
                base_margin,
                title_y,
                &slide.title,
                theme.title_size,
                &theme.on_accent.clone(),
                true,
                false,
                false,
                TextAlign::Left,
                w - 2 * base_margin,
            );
        } else {
            let title_align = if theme.title_center {
                TextAlign::Center
            } else {
                TextAlign::Left
            };
            self.text_line(
                content_x,
                title_y,
                &slide.title,
                theme.title_size,
                &theme.title_color.clone(),
                true,
                false,
                false,
                title_align,
                content_w,
            );
        }

        let underline_y = if matches!(layout.kind, LayoutKind::Clean) {
            let y = title_y + title_h + title_extra + 30000;
            self.rect(
                content_x,
                y + 18000,
                content_w,
                14000,
                &theme.divider.clone(),
            );
            self.rect(
                content_x,
                y,
                progress_width(content_w, num, total),
                50000,
                &theme.accent.clone(),
            );
            y
        } else {
            title_y + title_h + title_extra
        };

        let content_y_start = underline_y
            + if matches!(layout.kind, LayoutKind::Clean) {
                200000
            } else {
                280000
            };
        let footer_y = h - 400000;
        let content_max_y = footer_y - 100000;
        let content_h = content_max_y.saturating_sub(content_y_start);

        // Vertical alignment: measure the content block, discard the trial
        // render, then re-render shifted so it sits centred/bottom-aligned in
        // the available band. (Mirrors the SVG renderer's measure-then-offset.)
        // Column slides centre each column independently in the Columns arm, so
        // skip the whole-stack offset for them (else the text would double-shift).
        let has_columns = slide
            .blocks
            .iter()
            .any(|b| matches!(b, Block::Columns { .. }));
        let valign = slide.valign();
        if valign != "top" && !has_columns {
            let ops_mark = self.ops.len();
            let links_mark = self.links.len();
            let end_y = self.render_blocks(
                &slide.blocks,
                content_x,
                content_y_start,
                content_w,
                content_h,
                imgs,
            );
            self.ops.truncate(ops_mark);
            self.links.truncate(links_mark);
            let used = end_y.saturating_sub(content_y_start);
            let slack = content_h.saturating_sub(used);
            let offset = match valign {
                "center" => slack / 2,
                "bottom" => slack,
                _ => 0,
            };
            let _ = self.render_blocks(
                &slide.blocks,
                content_x,
                content_y_start + offset,
                content_w,
                content_h,
                imgs,
            );
        } else {
            let _ = self.render_blocks(
                &slide.blocks,
                content_x,
                content_y_start,
                content_w,
                content_h,
                imgs,
            );
        }

        if !layout.shows_sidebar() {
            // Logo replaces the deck-title text in the footer when present.
            if let Some((key, iw, ih)) = imgs.logo() {
                let logo_h = 280000_u32; // ~0.3"
                let logo_w = if ih > 0 {
                    ((logo_h as u64 * iw as u64) / ih as u64) as u32
                } else {
                    logo_h
                };
                let logo_y = footer_y - logo_h / 4;
                self.draw_image(key, content_x, logo_y, logo_w, logo_h, imgs);
            } else {
                self.text_line(
                    content_x,
                    footer_y,
                    deck_title,
                    1000,
                    &theme.muted_color.clone(),
                    false,
                    false,
                    false,
                    TextAlign::Left,
                    content_w.saturating_sub(800000),
                );
            }
            self.text_line(
                w - base_margin - 800000,
                footer_y,
                &format!("{} / {}", num, total),
                1000,
                &theme.muted_color.clone(),
                false,
                false,
                false,
                TextAlign::Right,
                800000,
            );
        }

        if layout.shows_corner_decoration() {
            self.rect(
                w - 280000,
                h - 280000,
                120000,
                120000,
                &theme.accent.clone(),
            );
        }
    }

    fn render_blocks(
        &mut self,
        blocks: &[Block],
        x: u32,
        y_start: u32,
        w: u32,
        _h_total: u32,
        imgs: &Imgs,
    ) -> u32 {
        let mut y = y_start;
        for block in blocks {
            match block {
                Block::Paragraph(runs) => {
                    let theme = self.theme;
                    let body_color = theme.body_color.clone();
                    let sz = (theme.body_size as f32 * self.cur_text_scale) as u32;
                    y = self.paragraph(runs, x, y, w, sz, &body_color, false, false);
                    y += 80000;
                }
                Block::Heading { level, runs } => {
                    let theme = self.theme;
                    let base = match level {
                        3 => theme.title_size - 400,
                        4 => theme.title_size - 600,
                        _ => theme.title_size - 800,
                    };
                    let sz = (base as f32 * self.cur_text_scale) as u32;
                    let title_color = theme.title_color.clone();
                    y = self.paragraph(runs, x, y, w, sz, &title_color, true, false);
                    y += 120000;
                }
                Block::List(items) => {
                    if items.len() > crate::theme::LONG_LIST_THRESHOLD && !self.theme.portrait {
                        let half = items.len().div_ceil(2);
                        let (l, r) = items.split_at(half);
                        let gap: u32 = 200000;
                        let col_w = (w.saturating_sub(gap)) / 2;
                        let left_y = self.render_list(l, x, y, col_w);
                        let right_y = self.render_list(r, x + col_w + gap, y, col_w);
                        y = left_y.max(right_y);
                    } else {
                        y = self.render_list(items, x, y, w);
                    }
                    y += 80000;
                }
                Block::CodeBlock {
                    lang,
                    title,
                    lines,
                    line_numbers,
                    start_line,
                    ..
                } => {
                    y = self.render_code_block(
                        lines,
                        title.as_deref(),
                        lang.as_deref(),
                        *line_numbers,
                        *start_line,
                        x,
                        y,
                        w,
                    );
                    y += 120000;
                }
                Block::Quote(paras) => {
                    y = self.render_quote(paras, x, y, w);
                    y += 80000;
                }
                Block::Callout { kind, body } => {
                    y = self.render_callout(kind, body, x, y, w);
                    y += 80000;
                }
                Block::Table {
                    headers,
                    rows,
                    aligns,
                } => {
                    y = self.render_table(headers, rows, aligns, x, y, w);
                    y += 80000;
                }
                Block::Columns { left, right } => {
                    let gap: u32 = 280000;
                    let avail = w.saturating_sub(gap);
                    // Honour the slide-level `width=` ratio; default to even.
                    let left_w = match self.cur_col_frac {
                        Some(f) => (avail as f32 * f) as u32,
                        None => avail / 2,
                    };
                    let right_w = avail.saturating_sub(left_w);
                    let rx = x + left_w + gap;
                    let start_y = y;
                    let factor = match self.cur_valign {
                        "center" => 0.5,
                        "bottom" => 1.0,
                        _ => 0.0,
                    };
                    if factor > 0.0 {
                        // Centre each column within the taller column's height
                        // (e.g. text beside an image), like HTML/SVG. Measure
                        // both via the truncate trick, then re-render at offsets.
                        let om = self.ops.len();
                        let lm = self.links.len();
                        let lh = self
                            .render_blocks(left, x, start_y, left_w, _h_total, imgs)
                            .saturating_sub(start_y);
                        self.ops.truncate(om);
                        self.links.truncate(lm);
                        let om2 = self.ops.len();
                        let lm2 = self.links.len();
                        let rh = self
                            .render_blocks(right, rx, start_y, right_w, _h_total, imgs)
                            .saturating_sub(start_y);
                        self.ops.truncate(om2);
                        self.links.truncate(lm2);
                        let maxh = lh.max(rh);
                        let loff = ((maxh.saturating_sub(lh)) as f32 * factor) as u32;
                        let roff = ((maxh.saturating_sub(rh)) as f32 * factor) as u32;
                        let ly =
                            self.render_blocks(left, x, start_y + loff, left_w, _h_total, imgs);
                        let ry =
                            self.render_blocks(right, rx, start_y + roff, right_w, _h_total, imgs);
                        y = ly.max(ry);
                    } else {
                        let left_y = self.render_blocks(left, x, start_y, left_w, _h_total, imgs);
                        let right_y =
                            self.render_blocks(right, rx, start_y, right_w, _h_total, imgs);
                        y = left_y.max(right_y);
                    }
                }
                Block::ColumnBreak => {}
                Block::Image {
                    src,
                    alt,
                    width_pct,
                    fit: _,
                    rounded: _,
                } => {
                    let effective_w = match width_pct {
                        Some(pct) => w * (*pct as u32) / 100,
                        None => w,
                    };
                    y = self.render_image_block(src, alt, x, y, effective_w, imgs);
                    y += 80000;
                }
                Block::Footnotes(items) => {
                    y = self.render_footnotes(items, x, y, w);
                    y += 80000;
                }
                Block::Cards { cards, cols } => {
                    y = self.render_cards(cards, *cols, x, y, w);
                    y += 80000;
                }
            }
        }
        y
    }

    fn render_footnotes(&mut self, items: &[ListItem], x: u32, y_start: u32, w: u32) -> u32 {
        let size = (self.theme.body_size as f32 * 0.7) as u32;
        let line_h = (size as f32 * 1.25 * EMU_PER_PT / 100.0) as u32;
        let muted = self.theme.muted_color.clone();
        let font = self.theme.body_font.clone();
        let _ = font;
        let mut y = y_start;
        // Thin divider above the footnotes for visual separation.
        self.rect(x, y, w.min(800000), 8000, &self.theme.divider.clone());
        y += 60000;
        for item in items {
            self.paragraph(&item.runs, x, y, w, size, &muted, false, false);
            y += line_h;
        }
        y
    }

    fn render_list(&mut self, items: &[ListItem], x: u32, y_start: u32, w: u32) -> u32 {
        let theme = self.theme;
        let body_color = theme.body_color.clone();
        let accent = theme.accent.clone();
        let size = (theme.body_size as f32 * self.cur_text_scale) as u32;
        let line_h = (size as f32 * 1.30 * EMU_PER_PT / 100.0) as u32;
        let mut y = y_start;
        let mut ordered_counters: Vec<u32> = Vec::new();
        for item in items {
            let lvl = item.level.min(8) as usize;
            while ordered_counters.len() <= lvl {
                ordered_counters.push(0);
            }
            ordered_counters[lvl] += 1;
            for c in &mut ordered_counters[lvl + 1..] {
                *c = 0;
            }
            // Task-list items carry their own ☐/☑ marker in the runs, so the
            // normal bullet is suppressed (and its gutter collapsed) to avoid
            // drawing both a bullet and a checkbox.
            let is_task = item.is_task();
            let bullet = if is_task {
                String::new()
            } else if item.ordered {
                format!("{}.", ordered_counters[lvl])
            } else {
                match lvl {
                    0 => "●".to_string(),
                    1 => "○".to_string(),
                    _ => "▪".to_string(),
                }
            };
            let indent_emu = (lvl as u32) * 300000;
            let bullet_x = x + indent_emu;
            // Measure the bullet so two-digit "10." / "11." don't overflow
            // the fixed gutter and bleed into the body text column.
            let bullet_pt = size as f32 / 100.0;
            let bullet_w_pt = text_width_pt(
                self.fonts,
                &bullet,
                font_index(false, false, false),
                bullet_pt,
            );
            let bullet_w_emu = (bullet_w_pt * EMU_PER_PT) as u32;
            let gutter_emu = if is_task {
                0
            } else {
                bullet_w_emu.max(180_000) + 120_000
            };
            self.text_line(
                bullet_x,
                y,
                &bullet,
                size,
                &accent,
                false,
                false,
                false,
                TextAlign::Left,
                gutter_emu,
            );
            let text_x = bullet_x + gutter_emu;
            let text_w = w.saturating_sub(text_x - x);
            let chars = item
                .runs
                .iter()
                .map(|r| r.text.chars().count())
                .sum::<usize>();
            // estimate height
            let estimated_lines =
                ((chars as f32 * (size as f32 / 100.0) * 0.55) / self.pt(text_w)).ceil() as u32;
            let next_y = self.paragraph(
                &item.runs,
                text_x,
                y,
                text_w,
                size,
                &body_color,
                false,
                false,
            );
            y = if next_y > y {
                next_y
            } else {
                y + line_h * estimated_lines.max(1)
            };
        }
        y
    }

    fn render_quote(&mut self, paras: &[Vec<Run>], x: u32, y_start: u32, w: u32) -> u32 {
        let theme = self.theme;
        let body_color = theme.body_color.clone();
        let accent = theme.accent.clone();
        let size = ((theme.body_size - 100) as f32 * self.cur_text_scale) as u32;
        let bar_x = x;
        let mut y = y_start;
        let start_y = y;
        for (i, runs) in paras.iter().enumerate() {
            if i > 0 {
                y += 120000;
            }
            y = self.paragraph(
                runs,
                x + 180000,
                y,
                w.saturating_sub(180000),
                size,
                &body_color,
                false,
                true,
            );
        }
        self.rect(bar_x, start_y, 60000, y.saturating_sub(start_y), &accent);
        y
    }

    /// Admonition box: tinted background, coloured left bar, icon+label
    /// heading, then the body paragraphs. Measure-then-draw so the box wraps
    /// the content.
    fn render_callout(
        &mut self,
        kind: &str,
        paras: &[Vec<Run>],
        x: u32,
        y_start: u32,
        w: u32,
    ) -> u32 {
        let accent = callout_color(kind).to_string();
        let (icon, label) = callout_label(kind);
        let body_color = self.theme.body_color.clone();
        let size = (self.theme.body_size.saturating_sub(100) as f32 * self.cur_text_scale) as u32;
        let label_size = (self.theme.body_size as f32 * self.cur_text_scale) as u32;
        let pad: u32 = 150000;
        let bar_w: u32 = 60000;
        let inner_x = x + bar_w + pad;
        let inner_w = w.saturating_sub(bar_w + pad * 2);
        let label_h = Self::line_h_emu(label_size);

        // Lay the inner content out once to discover the box height, then
        // discard the trial ops/links and draw the box behind a real pass.
        let body_layout = |s: &mut Self, top: u32| -> u32 {
            let mut yy = top + pad + label_h; // label line sits at top+pad
            for (i, runs) in paras.iter().enumerate() {
                if i > 0 {
                    yy += 80000;
                }
                yy = s.paragraph(runs, inner_x, yy, inner_w, size, &body_color, false, false);
            }
            yy + pad
        };

        let ops_mark = self.ops.len();
        let links_mark = self.links.len();
        let bottom = body_layout(self, y_start);
        self.ops.truncate(ops_mark);
        self.links.truncate(links_mark);
        let box_h = bottom.saturating_sub(y_start);

        let tint = light_tint(callout_color(kind));
        self.rect(x, y_start, w, box_h, &tint);
        self.rect(x, y_start, bar_w, box_h, &accent);
        self.text_line(
            inner_x,
            y_start + pad,
            &format!("{icon} {label}"),
            label_size,
            &accent,
            true,
            false,
            false,
            TextAlign::Left,
            inner_w,
        );
        body_layout(self, y_start);
        y_start + box_h
    }

    /// Fixed N-column grid of bordered cards (title + body), wrapping rows.
    fn render_cards(&mut self, cards: &[Card], cols: u8, x: u32, y_start: u32, w: u32) -> u32 {
        let n = (cols as usize).max(1);
        let gap: u32 = 180000;
        let pad: u32 = 140000;
        let col_w = w.saturating_sub(gap * (n as u32 - 1)) / n as u32;
        let inner_w = col_w.saturating_sub(pad * 2);
        let title_color = self.theme.title_color.clone();
        let body_color = self.theme.body_color.clone();
        let border = self.theme.divider.clone();
        let title_size = (self.theme.body_size as f32 * self.cur_text_scale) as u32;
        let body_size =
            (self.theme.body_size.saturating_sub(100) as f32 * self.cur_text_scale) as u32;
        let title_h = Self::line_h_emu(title_size);

        // Render one card's inner content at (cx, top); returns the bottom y.
        let card_inner = |s: &mut Self, card: &Card, cx: u32, top: u32| -> u32 {
            let ix = cx + pad;
            let mut yy = top + pad;
            let lines = s.text_line(
                ix,
                yy,
                &card.title,
                title_size,
                &title_color,
                true,
                false,
                false,
                TextAlign::Left,
                inner_w,
            );
            yy += lines as u32 * title_h + 40000;
            yy = s.paragraph(
                &card.body,
                ix,
                yy,
                inner_w,
                body_size,
                &body_color,
                false,
                false,
            );
            yy + pad
        };

        let mut y = y_start;
        for row in cards.chunks(n) {
            // Measure the tallest card in this row so the boxes align.
            let mut row_h = 0u32;
            for (i, card) in row.iter().enumerate() {
                let cx = x + i as u32 * (col_w + gap);
                let ops_mark = self.ops.len();
                let links_mark = self.links.len();
                let bottom = card_inner(self, card, cx, y);
                self.ops.truncate(ops_mark);
                self.links.truncate(links_mark);
                row_h = row_h.max(bottom.saturating_sub(y));
            }
            // Draw boxes then content.
            for (i, card) in row.iter().enumerate() {
                let cx = x + i as u32 * (col_w + gap);
                self.rect(cx, y, col_w, row_h, &border);
                self.rect(
                    cx + 8000,
                    y + 8000,
                    col_w - 16000,
                    row_h - 16000,
                    &self.theme.bg.clone(),
                );
                card_inner(self, card, cx, y);
            }
            y += row_h + gap;
        }
        y.saturating_sub(gap)
    }

    fn render_code_block(
        &mut self,
        lines: &[String],
        title: Option<&str>,
        lang: Option<&str>,
        line_numbers: bool,
        start_line: usize,
        x: u32,
        y_start: u32,
        w: u32,
    ) -> u32 {
        let theme = self.theme;
        let title_h: u32 = if title.is_some() || lang.is_some() {
            320000
        } else {
            0
        };
        let code_size = theme.code_size;
        let line_h = (code_size as f32 * 1.30 * EMU_PER_PT / 100.0) as u32;
        let pad = 180000;
        let _body_h = line_h * (lines.len() as u32).max(1) + 2 * pad;

        if title_h > 0 {
            self.rect(x, y_start, w, title_h, &theme.divider.clone());
            if let Some(t) = title {
                self.text_line(
                    x + 220000,
                    y_start + 70000,
                    t,
                    1200,
                    &theme.title_color.clone(),
                    true,
                    false,
                    true,
                    TextAlign::Left,
                    w - 440000,
                );
                if let Some(l) = lang {
                    let title_w_pt = text_width_pt(self.fonts, t, FONT_COUR_BOLD, 12.0);
                    let label_x = x + 220000 + (title_w_pt * EMU_PER_PT) as u32 + 200000;
                    self.text_line(
                        label_x,
                        y_start + 80000,
                        &format!("· {}", l),
                        1100,
                        &theme.muted_color.clone(),
                        false,
                        false,
                        false,
                        TextAlign::Left,
                        w / 2,
                    );
                }
            } else if let Some(l) = lang {
                self.text_line(
                    x + 220000,
                    y_start + 70000,
                    l,
                    1200,
                    &theme.muted_color.clone(),
                    true,
                    false,
                    false,
                    TextAlign::Left,
                    w - 440000,
                );
            }
        }

        let highlighted: Vec<Vec<Token>> = syntax::tokenize(lines, lang);

        let last_line = start_line.saturating_add(lines.len().saturating_sub(1));
        let gutter_w = if line_numbers {
            Some(last_line.to_string().len())
        } else {
            None
        };

        // Auto-scale code font when the widest line would overflow the box.
        let base_code_size = theme.code_size;
        let mut max_line_content_pt: f32 = 0.0;
        for line_tokens in &highlighted {
            let mut line_w = 0.0_f32;
            for token in line_tokens {
                if token.text.is_empty() {
                    continue;
                }
                let style = theme.syntax_style(token.kind);
                let font_idx = font_index(style.bold, style.italic, true);
                line_w += text_width_pt(
                    self.fonts,
                    &token.text,
                    font_idx,
                    base_code_size as f32 / 100.0,
                );
            }
            if line_w > max_line_content_pt {
                max_line_content_pt = line_w;
            }
        }
        let gutter_text_w_pt = if let Some(width) = gutter_w {
            let n = format!("{:>w$}", last_line, w = width);
            text_width_pt(self.fonts, &n, FONT_COUR, base_code_size as f32 / 100.0)
        } else {
            0.0
        };
        let gutter_gap_pt = if gutter_w.is_some() {
            self.pt(240000)
        } else {
            0.0
        };
        let inner_w_pt =
            (self.pt(w) - 2.0 * self.pt(pad) - gutter_text_w_pt - gutter_gap_pt).max(20.0);
        let code_scale: f32 = if max_line_content_pt > inner_w_pt && max_line_content_pt > 0.0 {
            (inner_w_pt / max_line_content_pt).max(0.55)
        } else {
            1.0
        };
        let code_size = ((base_code_size as f32) * code_scale).max(700.0) as u32;
        let line_h_scaled = (code_size as f32 * 1.30 * EMU_PER_PT / 100.0) as u32;
        let code_size_pt = code_size as f32 / 100.0;

        // For lines still too wide after scaling, wrap them across multiple
        // visual rows. Each visual row carries (tokens, is_continuation).
        let inner_after_gutter_pt = (self.pt(w)
            - 2.0 * self.pt(pad)
            - text_width_pt(
                self.fonts,
                &format!("{:>w$}", last_line, w = gutter_w.unwrap_or(0)),
                FONT_COUR,
                code_size_pt,
            )
            - gutter_gap_pt)
            .max(20.0);
        let token_width = |t: &Token| -> f32 {
            let style = theme.syntax_style(t.kind);
            text_width_pt(
                self.fonts,
                &t.text,
                font_index(style.bold, style.italic, true),
                code_size_pt,
            )
        };
        let split_token_at_width = |t: &Token, budget_pt: f32| -> Option<(Token, Token)> {
            if t.text.is_empty() || budget_pt <= 0.0 {
                return None;
            }
            let style = theme.syntax_style(t.kind);
            let font_idx = font_index(style.bold, style.italic, true);
            let mut acc = 0.0_f32;
            let mut split_idx = 0;
            for (i, c) in t.text.char_indices() {
                let cw = text_width_pt(self.fonts, &c.to_string(), font_idx, code_size_pt);
                if acc + cw > budget_pt {
                    break;
                }
                acc += cw;
                split_idx = i + c.len_utf8();
            }
            if split_idx == 0 || split_idx >= t.text.len() {
                return None;
            }
            let (a, b) = t.text.split_at(split_idx);
            Some((
                Token {
                    text: a.to_string(),
                    kind: t.kind,
                },
                Token {
                    text: b.to_string(),
                    kind: t.kind,
                },
            ))
        };

        let mut visual_lines: Vec<(Vec<Token>, bool)> = Vec::new(); // (tokens, is_continuation)
        for orig in &highlighted {
            let mut current: Vec<Token> = Vec::new();
            let mut current_w = 0.0_f32;
            let mut is_continuation = false;
            for token in orig {
                if token.text.is_empty() {
                    continue;
                }
                let tw = token_width(token);
                if !current.is_empty() && current_w + tw > inner_after_gutter_pt {
                    visual_lines.push((std::mem::take(&mut current), is_continuation));
                    current_w = 0.0;
                    is_continuation = true;
                    // Skip a leading all-whitespace token on a continuation line.
                    if token.text.chars().all(|c| c == ' ' || c == '\t') {
                        continue;
                    }
                }
                // If the single token itself is wider than the budget, force-split it.
                let mut remaining = token.clone();
                let mut remaining_w = token_width(&remaining);
                while !remaining.text.is_empty()
                    && current_w + remaining_w > inner_after_gutter_pt
                    && current_w == 0.0
                {
                    let budget = inner_after_gutter_pt - current_w;
                    match split_token_at_width(&remaining, budget) {
                        Some((head, tail)) => {
                            current.push(head);
                            visual_lines.push((std::mem::take(&mut current), is_continuation));
                            is_continuation = true;
                            current_w = 0.0;
                            remaining = tail;
                            remaining_w = token_width(&remaining);
                        }
                        None => break,
                    }
                }
                if !remaining.text.is_empty() {
                    current.push(remaining);
                    current_w += remaining_w;
                }
            }
            if current.is_empty() && !is_continuation {
                // Preserve empty lines.
                visual_lines.push((Vec::new(), false));
            } else if !current.is_empty() {
                visual_lines.push((current, is_continuation));
            }
        }

        let body_h_scaled = line_h_scaled * (visual_lines.len() as u32).max(1) + 2 * pad;
        let total_h = body_h_scaled + title_h;

        let body_y = y_start + title_h;
        self.rect(x, body_y, w, body_h_scaled, &theme.code_bg.clone());

        let muted = theme.muted_color.clone();
        let continuation_indent_emu: u32 = (2.0 * 0.6 * code_size_pt * EMU_PER_PT) as u32;
        let mut orig_line_idx = start_line.saturating_sub(1);
        for (visual_idx, (line_tokens, is_continuation)) in visual_lines.iter().enumerate() {
            let line_y = body_y + pad + visual_idx as u32 * line_h_scaled;
            let mut text_x = x + pad;
            if let Some(width) = gutter_w {
                let gutter_text = if *is_continuation {
                    " ".repeat(width)
                } else {
                    orig_line_idx += 1;
                    format!("{:>w$}", orig_line_idx, w = width)
                };
                let gutter_pt = text_width_pt(self.fonts, &gutter_text, FONT_COUR, code_size_pt);
                let gutter_emu = (gutter_pt * EMU_PER_PT) as u32;
                if !is_continuation {
                    self.text_line(
                        text_x,
                        line_y,
                        &gutter_text,
                        code_size,
                        &muted,
                        false,
                        false,
                        true,
                        TextAlign::Left,
                        // Small slack: gutter_emu is truncated from gutter_pt, so
                        // passing it verbatim leaves the number a hair too wide
                        // and text_line wraps multi-digit numbers ("10" → 1/0).
                        gutter_emu + 40000,
                    );
                }
                text_x += gutter_emu + 240000;
            } else if !*is_continuation {
                orig_line_idx += 1;
            }
            if *is_continuation {
                text_x += continuation_indent_emu;
            }
            let mut cursor_emu = text_x;
            for token in line_tokens {
                if token.text.is_empty() {
                    continue;
                }
                let style = theme.syntax_style(token.kind);
                let color = style.color.clone();
                // 0 disables text_line's internal wrap — the visual_lines
                // pass above has already broken oversized lines into
                // per-row tokens. Letting text_line wrap again would emit
                // an extra Tj at `baseline - size_pt * 1.25`, which is a
                // different stride than `line_h_scaled` and collides with
                // the next gutter row.
                self.text_line(
                    cursor_emu,
                    line_y,
                    &token.text,
                    code_size,
                    &color,
                    style.bold,
                    style.italic,
                    true,
                    TextAlign::Left,
                    0,
                );
                let w_pt = text_width_pt(
                    self.fonts,
                    &token.text,
                    font_index(style.bold, style.italic, true),
                    code_size_pt,
                );
                cursor_emu += (w_pt * EMU_PER_PT) as u32;
            }
        }
        y_start + total_h
    }

    fn render_table(
        &mut self,
        headers: &[Vec<Run>],
        rows: &[Vec<Vec<Run>>],
        aligns: &[crate::ir::ColumnAlign],
        x: u32,
        y_start: u32,
        w: u32,
    ) -> u32 {
        let cols = headers
            .len()
            .max(rows.iter().map(|r| r.len()).max().unwrap_or(0));
        if cols == 0 {
            return y_start;
        }
        let theme = self.theme;

        let base_header_size = theme.body_size as i32 - 200;
        let base_body_size = theme.body_size as i32 - 400;
        let default_pad_x = 120000_u32;

        let header_font_idx = font_index(true, false, false);
        let body_font_idx = font_index(false, false, false);

        // Longest single token (no break possible) per column, at base font size.
        let longest_token_pt = |size_centipt: i32| -> Vec<f32> {
            let size_pt = size_centipt as f32 / 100.0;
            (0..cols)
                .map(|c| {
                    let mut longest = 0.0_f32;
                    if let Some(hr) = headers.get(c) {
                        for r in hr {
                            for tok in r.text.split_whitespace() {
                                let tw = text_width_pt(self.fonts, tok, header_font_idx, size_pt);
                                if tw > longest {
                                    longest = tw;
                                }
                            }
                        }
                    }
                    for row in rows {
                        if let Some(cell) = row.get(c) {
                            for r in cell {
                                for tok in r.text.split_whitespace() {
                                    let tw = text_width_pt(self.fonts, tok, body_font_idx, size_pt);
                                    if tw > longest {
                                        longest = tw;
                                    }
                                }
                            }
                        }
                    }
                    longest
                })
                .collect()
        };

        // Auto-scale font when even longest tokens + padding can't fit at base size.
        // Use header size for the scale check (it's the largest).
        let base_mins_pt: Vec<f32> = longest_token_pt(base_header_size);
        let base_pad_pt = self.pt(default_pad_x);
        let natural_total_pt: f32 =
            base_mins_pt.iter().sum::<f32>() + cols as f32 * 2.0 * base_pad_pt;
        let available_pt = self.pt(w);
        let table_scale: f32 = if natural_total_pt > available_pt && natural_total_pt > 0.0 {
            (available_pt / natural_total_pt).max(0.55)
        } else {
            1.0
        };

        let header_size = ((base_header_size as f32) * table_scale).max(800.0) as u32;
        let body_size = ((base_body_size as f32) * table_scale).max(700.0) as u32;
        let pad_x = ((default_pad_x as f32) * table_scale.max(0.7)) as u32;
        let pad_pt = self.pt(pad_x);
        let line_h_header = (header_size as f32 * 1.30 * EMU_PER_PT / 100.0) as u32;
        let line_h_body = (body_size as f32 * 1.30 * EMU_PER_PT / 100.0) as u32;
        let pad_y = (90000_u32 as f32 * table_scale.max(0.7)) as u32;

        // Allocate column widths from longest-token widths at the SCALED font.
        // Each column gets its minimum (longest token + 2*pad), then any
        // leftover space is distributed proportional to character count
        // (so the bulkiest column gets the most breathing room).
        let scaled_mins_pt: Vec<f32> = (0..cols)
            .map(|c| {
                let mut longest = 0.0_f32;
                if let Some(hr) = headers.get(c) {
                    for r in hr {
                        for tok in r.text.split_whitespace() {
                            let tw = text_width_pt(
                                self.fonts,
                                tok,
                                header_font_idx,
                                header_size as f32 / 100.0,
                            );
                            if tw > longest {
                                longest = tw;
                            }
                        }
                    }
                }
                for row in rows {
                    if let Some(cell) = row.get(c) {
                        for r in cell {
                            for tok in r.text.split_whitespace() {
                                let tw = text_width_pt(
                                    self.fonts,
                                    tok,
                                    body_font_idx,
                                    body_size as f32 / 100.0,
                                );
                                if tw > longest {
                                    longest = tw;
                                }
                            }
                        }
                    }
                }
                longest
            })
            .collect();
        let mins_with_pad_pt: Vec<f32> = scaled_mins_pt.iter().map(|m| m + 2.0 * pad_pt).collect();
        let mins_sum_pt: f32 = mins_with_pad_pt.iter().sum();
        let extra_pt = (available_pt - mins_sum_pt).max(0.0);
        let content_weights: Vec<f32> = (0..cols)
            .map(|c| {
                let mut chars: usize = 0;
                if let Some(hr) = headers.get(c) {
                    chars += hr.iter().map(|r| r.text.chars().count()).sum::<usize>();
                }
                for row in rows {
                    if let Some(cell) = row.get(c) {
                        chars += cell.iter().map(|r| r.text.chars().count()).sum::<usize>();
                    }
                }
                (chars as f32).max(1.0)
            })
            .collect();
        let total_content_weight: f32 = content_weights.iter().sum();
        let col_widths_pt: Vec<f32> = mins_with_pad_pt
            .iter()
            .zip(content_weights.iter())
            .map(|(base, cw)| {
                let share = if total_content_weight > 0.0 {
                    extra_pt * cw / total_content_weight
                } else {
                    extra_pt / cols as f32
                };
                base + share
            })
            .collect();
        let mut col_widths: Vec<u32> = col_widths_pt
            .iter()
            .map(|wp| (wp * EMU_PER_PT) as u32)
            .collect();
        let assigned: u32 = col_widths.iter().sum();
        if assigned > 0 && assigned != w {
            let last = col_widths.len() - 1;
            if assigned > w {
                col_widths[last] = col_widths[last].saturating_sub(assigned - w);
            } else {
                col_widths[last] += w - assigned;
            }
        }
        let col_x: Vec<u32> = {
            let mut v = Vec::with_capacity(cols);
            let mut acc = 0u32;
            for cw in &col_widths {
                v.push(acc);
                acc += cw;
            }
            v
        };

        // Wrap headers + measure header row height.
        let header_wrapped: Vec<Vec<Vec<Run>>> = (0..cols)
            .map(|c| {
                let runs = headers.get(c).map(|r| r.as_slice()).unwrap_or(&[]);
                let inner_w_pt = self.pt(col_widths[c].saturating_sub(2 * pad_x));
                wrap_runs(
                    self.fonts,
                    runs,
                    inner_w_pt,
                    header_size as f32 / 100.0,
                    true,
                    false,
                )
            })
            .collect();
        let header_lines = header_wrapped
            .iter()
            .map(|l| l.len().max(1))
            .max()
            .unwrap_or(1);
        let header_h = (line_h_header * header_lines as u32) + 2 * pad_y;

        // Wrap each body row + compute its height.
        let row_wrapped: Vec<Vec<Vec<Vec<Run>>>> = rows
            .iter()
            .map(|row| {
                (0..cols)
                    .map(|c| {
                        let runs = row.get(c).map(|r| r.as_slice()).unwrap_or(&[]);
                        let inner_w_pt = self.pt(col_widths[c].saturating_sub(2 * pad_x));
                        wrap_runs(
                            self.fonts,
                            runs,
                            inner_w_pt,
                            body_size as f32 / 100.0,
                            false,
                            false,
                        )
                    })
                    .collect()
            })
            .collect();
        let row_heights: Vec<u32> = row_wrapped
            .iter()
            .map(|cells| {
                let max_lines = cells.iter().map(|l| l.len().max(1)).max().unwrap_or(1);
                (line_h_body * max_lines as u32) + 2 * pad_y
            })
            .collect();

        // Render header.
        for c in 0..cols {
            self.rect(
                x + col_x[c],
                y_start,
                col_widths[c],
                header_h,
                &theme.accent.clone(),
            );
            for (i, line) in header_wrapped[c].iter().enumerate() {
                let lw = runs_width_emu(self.fonts, line, header_size, true);
                let tx = aligned_cell_x(
                    aligns.get(c).copied(),
                    x + col_x[c],
                    col_widths[c],
                    pad_x,
                    lw,
                );
                self.text_runs_line(
                    line,
                    tx,
                    y_start + pad_y + i as u32 * line_h_header,
                    header_size,
                    &theme.on_accent.clone(),
                    true,
                    false,
                );
            }
        }
        // Render body rows.
        let mut ry = y_start + header_h;
        let table_band_bg = theme.table_band_bg();
        for (i, row_cells) in row_wrapped.iter().enumerate() {
            let rh = row_heights[i];
            let banded = i % 2 == 1;
            let bg = if banded {
                table_band_bg.clone()
            } else {
                theme.bg.clone()
            };
            for c in 0..cols {
                self.rect(x + col_x[c], ry, col_widths[c], rh, &bg);
                for (l, line) in row_cells[c].iter().enumerate() {
                    let lw = runs_width_emu(self.fonts, line, body_size, false);
                    let tx = aligned_cell_x(
                        aligns.get(c).copied(),
                        x + col_x[c],
                        col_widths[c],
                        pad_x,
                        lw,
                    );
                    self.text_runs_line(
                        line,
                        tx,
                        ry + pad_y + l as u32 * line_h_body,
                        body_size,
                        &theme.body_color.clone(),
                        false,
                        false,
                    );
                }
            }
            ry += rh;
        }
        ry
    }

    fn render_image_block(
        &mut self,
        src: &str,
        alt: &str,
        x: u32,
        y_start: u32,
        w: u32,
        imgs: &Imgs,
    ) -> u32 {
        // Image alt text is an accessibility description, not a display
        // caption — HTML keeps it as the `<img alt>` and SVG omits it, so the
        // PDF no longer paints it under the image either (kept consistent across
        // renderers). `alt` still drives the placeholder text when the image is
        // missing, via fit_image_for_block/image_x_for_block below.
        let caption_alt = "";
        let caption_h = if caption_alt.is_empty() { 0 } else { 260000 };
        // Cap image to 65% of the slide height so titles and surrounding
        // content stay visible. Deriving from `theme.slide_h` keeps this
        // sane on portrait + paper-size aspects, where a hardcoded EMU
        // ceiling would either squash images or let them swallow the slide.
        let max_h: u32 = self.theme.slide_h * crate::theme::IMAGE_MAX_HEIGHT_FRACTION_NUM
            / crate::theme::IMAGE_MAX_HEIGHT_FRACTION_DEN;
        let max_image_h = max_h.saturating_sub(caption_h + 80000);
        let (display_w, display_h) = if let Some((iw, ih)) = imgs.dims(src) {
            fit_image_for_block(src, alt, iw, ih, w, max_image_h, self.theme.slide_h)
        } else {
            (w, max_image_h)
        };
        let img_x = image_x_for_block(src, alt, x, w, display_w);
        self.draw_image(src, img_x, y_start, display_w, display_h, imgs);
        let mut y = y_start + display_h;
        if !caption_alt.is_empty() {
            y += 80000;
            let muted = self.theme.muted_color.clone();
            self.text_line(
                x,
                y,
                caption_alt,
                1300,
                &muted,
                false,
                true,
                false,
                TextAlign::Center,
                w,
            );
            y += caption_h;
        }
        y
    }
}

// ---------------------------------------------------------------------------
// Text wrapping
// ---------------------------------------------------------------------------

/// Hard-break a single token (no spaces) into chunks that each fit `max_w_pt`,
/// for emergency wrapping of long unbreakable tokens (URLs, hashes, CamelCase)
/// that would otherwise run off the slide edge.
fn break_token_to_width(
    fonts: &PdfFonts,
    tok: &str,
    font_idx: usize,
    size_pt: f32,
    max_w_pt: f32,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0.0;
    for ch in tok.chars() {
        let cw = text_width_pt(fonts, ch.encode_utf8(&mut [0u8; 4]), font_idx, size_pt);
        if !cur.is_empty() && cur_w + cw > max_w_pt {
            out.push(std::mem::take(&mut cur));
            cur_w = 0.0;
        }
        cur.push(ch);
        cur_w += cw;
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

fn wrap_text_simple(
    fonts: &PdfFonts,
    text: &str,
    font_idx: usize,
    size_pt: f32,
    max_w_pt: f32,
) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_w: f32 = 0.0;

    let mut tokens: Vec<String> = Vec::new();
    let mut buf = String::new();
    let mut in_space = false;
    for ch in text.chars() {
        let is_sp = ch == ' ' || ch == '\t';
        if buf.is_empty() {
            buf.push(ch);
            in_space = is_sp;
        } else if is_sp == in_space {
            buf.push(ch);
        } else {
            tokens.push(std::mem::take(&mut buf));
            buf.push(ch);
            in_space = is_sp;
        }
    }
    if !buf.is_empty() {
        tokens.push(buf);
    }

    for tok in tokens {
        let tok_w = text_width_pt(fonts, &tok, font_idx, size_pt);
        let only_space = tok.chars().all(|c| c == ' ' || c == '\t');
        // A token wider than the whole line can't be placed as-is — hard-break
        // it so it wraps instead of overflowing the edge.
        if !only_space && tok_w > max_w_pt && max_w_pt > 0.0 {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
                current_w = 0.0;
            }
            let chunks = break_token_to_width(fonts, &tok, font_idx, size_pt, max_w_pt);
            let n = chunks.len();
            for (i, chunk) in chunks.into_iter().enumerate() {
                if i + 1 < n {
                    lines.push(chunk);
                } else {
                    current_w = text_width_pt(fonts, &chunk, font_idx, size_pt);
                    current = chunk;
                }
            }
            continue;
        }
        if !current.is_empty() && current_w + tok_w > max_w_pt && !only_space {
            lines.push(std::mem::take(&mut current));
            current_w = 0.0;
        }
        if current.is_empty() && only_space {
            continue;
        }
        current.push_str(&tok);
        current_w += tok_w;
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn wrap_runs(
    fonts: &PdfFonts,
    runs: &[Run],
    max_w_pt: f32,
    size_pt: f32,
    base_bold: bool,
    base_italic: bool,
) -> Vec<Vec<Run>> {
    // Greedy word-based wrap. Each run's text is split into tokens (word + trailing space).
    let mut lines: Vec<Vec<Run>> = vec![Vec::new()];
    let mut cur_width: f32 = 0.0;

    for r in runs {
        let bold = base_bold || r.bold;
        let italic = base_italic || r.italic;
        let mono = r.code;
        let font_idx = font_index(bold, italic, mono);
        // Tokenize keeping spaces.
        let mut tokens: Vec<String> = Vec::new();
        let mut buf = String::new();
        let mut in_space = false;
        for ch in r.text.chars() {
            let is_sp = ch == ' ';
            if buf.is_empty() {
                buf.push(ch);
                in_space = is_sp;
            } else if is_sp == in_space {
                buf.push(ch);
            } else {
                tokens.push(std::mem::take(&mut buf));
                buf.push(ch);
                in_space = is_sp;
            }
        }
        if !buf.is_empty() {
            tokens.push(buf);
        }
        for tok in tokens {
            let tok_w = text_width_pt(fonts, &tok, font_idx, size_pt);
            let only_space = tok.chars().all(|c| c == ' ');
            let mk_run = |text: String| Run {
                text,
                bold: r.bold,
                italic: r.italic,
                code: r.code,
                strike: r.strike,
                link: r.link.clone(),
            };
            // Hard-break a token wider than the whole line so it wraps instead
            // of overflowing the slide edge.
            if !only_space && tok_w > max_w_pt && max_w_pt > 0.0 {
                if !lines.last().unwrap().is_empty() {
                    lines.push(Vec::new());
                    cur_width = 0.0;
                }
                let chunks = break_token_to_width(fonts, &tok, font_idx, size_pt, max_w_pt);
                let n = chunks.len();
                for (i, chunk) in chunks.into_iter().enumerate() {
                    let cw = text_width_pt(fonts, &chunk, font_idx, size_pt);
                    lines.last_mut().unwrap().push(mk_run(chunk));
                    if i + 1 < n {
                        lines.push(Vec::new());
                        cur_width = 0.0;
                    } else {
                        cur_width = cw;
                    }
                }
                continue;
            }
            if cur_width + tok_w > max_w_pt && !lines.last().unwrap().is_empty() && !only_space {
                lines.push(Vec::new());
                cur_width = 0.0;
                // Drop leading whitespace at start of new line.
                if tok.chars().all(|c| c == ' ') {
                    continue;
                }
            }
            lines.last_mut().unwrap().push(mk_run(tok));
            cur_width += tok_w;
        }
    }
    lines
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum TextAlign {
    Left,
    Center,
    Right,
}

/// Glyph-advance source for the math layout engine backed by the PDF's
/// embedded faces. Measuring each glyph in the face that will actually render
/// it (via [`text_width_pt`], the same fallback selection the writer uses)
/// keeps the reserved layout width equal to the drawn advance even when
/// `--font` swaps DejaVu for a brand face. A non-positive advance (combining
/// marks) defers to the built-in DejaVu table.
struct PdfMathMetrics<'a> {
    fonts: &'a PdfFonts,
    font_idx: usize,
}

impl crate::math::GlyphMetrics for PdfMathMetrics<'_> {
    fn advance_em(&self, ch: char, bold: bool) -> f32 {
        // Bold text is drawn in the bold sans face, which has its own
        // advances — measure that face so a bold run reserves the right room.
        let font_idx = if bold { FONT_HELV_BOLD } else { self.font_idx };
        let mut buf = [0u8; 4];
        let s = ch.encode_utf8(&mut buf);
        let w = text_width_pt(self.fonts, s, font_idx, 1.0);
        if w > 0.0 {
            w
        } else {
            crate::math::DejaVuMetrics.advance_em(ch, bold)
        }
    }
}

fn math_markup_line_layouts(
    lines: &[String],
    metrics: &dyn crate::math::GlyphMetrics,
) -> Vec<Option<crate::math::MathTextLayout>> {
    lines
        .iter()
        .map(|line| {
            if line.trim().is_empty() {
                None
            } else {
                Some(crate::math::layout_markup_text_with(line, 100, metrics))
            }
        })
        .collect()
}

fn pack_math_markup_line_layouts(
    lines: &[String],
    target_width: f32,
    metrics: &dyn crate::math::GlyphMetrics,
) -> Vec<Option<crate::math::MathTextLayout>> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut current_layout = None;

    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if let Some(layout) = current_layout.take() {
                out.push(Some(layout));
                current.clear();
            }
            out.push(None);
            continue;
        }

        if current.is_empty() {
            current.push_str(trimmed);
            current_layout = Some(crate::math::layout_markup_text_with(&current, 100, metrics));
            continue;
        }

        let candidate = format!("{current} {trimmed}");
        let candidate_layout = crate::math::layout_markup_text_with(&candidate, 100, metrics);
        if candidate_layout.width <= target_width {
            current = candidate;
            current_layout = Some(candidate_layout);
        } else {
            if let Some(layout) = current_layout.take() {
                out.push(Some(layout));
            }
            current.clear();
            current.push_str(trimmed);
            current_layout = Some(crate::math::layout_markup_text_with(&current, 100, metrics));
        }
    }

    if let Some(layout) = current_layout {
        out.push(Some(layout));
    }
    out
}

fn fit_packed_math_markup_line_layouts(
    lines: &[String],
    raw_layouts: Vec<Option<crate::math::MathTextLayout>>,
    base_size: f32,
    gap: f32,
    page_ratio: f32,
    metrics: &dyn crate::math::GlyphMetrics,
) -> Vec<Option<crate::math::MathTextLayout>> {
    let (raw_max_line_w, raw_total_h) = math_markup_metrics(&raw_layouts, base_size, gap);
    let raw_ratio = raw_max_line_w / raw_total_h.max(1.0);
    let desired_ratio = page_ratio * 0.72;
    if lines.len() <= 20 || raw_ratio >= desired_ratio * 0.75 {
        return raw_layouts;
    }

    let mut best = raw_layouts.clone();
    let mut best_err = (raw_ratio - desired_ratio).abs();
    let mut low = raw_max_line_w;
    let mut high = (raw_total_h * desired_ratio * 2.0).max(raw_max_line_w * 1.05);

    for _ in 0..9 {
        let target = (low + high) / 2.0;
        let packed = pack_math_markup_line_layouts(lines, target, metrics);
        let (packed_w, packed_h) = math_markup_metrics(&packed, base_size, gap);
        let ratio = packed_w / packed_h.max(1.0);
        let err = (ratio - desired_ratio).abs();
        if err < best_err {
            best = packed;
            best_err = err;
        }
        if ratio < desired_ratio {
            low = target;
        } else {
            high = target;
        }
    }

    best
}

fn math_markup_metrics(
    layouts: &[Option<crate::math::MathTextLayout>],
    base_size: f32,
    gap: f32,
) -> (f32, f32) {
    let max_line_w = layouts
        .iter()
        .filter_map(|layout| layout.as_ref().map(|layout| layout.width))
        .fold(1.0_f32, f32::max);
    let total_h = layouts
        .iter()
        .map(|layout| {
            layout
                .as_ref()
                .map(|layout| layout.height)
                .unwrap_or(base_size * 0.65)
        })
        .sum::<f32>()
        + gap * layouts.len().saturating_sub(1) as f32;
    (max_line_w, total_h)
}

fn hex_to_rgb_f(hex: &str) -> (f32, f32, f32) {
    if hex.len() != 6 {
        return (0.0, 0.0, 0.0);
    }
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
    (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0)
}

/// Accent colour (no `#`) for a callout kind. Mirrors the SVG/HTML palette.
fn callout_color(kind: &str) -> &'static str {
    match kind {
        "tip" => "22C55E",
        "important" => "A855F7",
        "warning" => "F59E0B",
        "caution" => "EF4444",
        _ => "3B82F6",
    }
}

/// Icon + display label for a callout kind.
fn callout_label(kind: &str) -> (&'static str, &'static str) {
    match kind {
        "tip" => ("\u{1F4A1}", "Tip"),
        "important" => ("\u{2757}", "Important"),
        "warning" => ("\u{26A0}", "Warning"),
        "caution" => ("\u{1F6D1}", "Caution"),
        _ => ("\u{2139}", "Note"),
    }
}

/// Light background tint for a callout: the accent mixed ~14% over white.
fn light_tint(hex: &str) -> String {
    let (r, g, b) = hex_to_rgb_f(hex);
    let mix = |c: f32| 1.0 - (1.0 - c) * 0.14;
    format!(
        "{:02X}{:02X}{:02X}",
        (mix(r) * 255.0) as u8,
        (mix(g) * 255.0) as u8,
        (mix(b) * 255.0) as u8,
    )
}

fn fit_image(iw: u32, ih: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    if iw == 0 || ih == 0 {
        return (max_w, max_h);
    }
    let iw = iw as u64;
    let ih = ih as u64;
    let mw = max_w as u64;
    let mh = max_h as u64;
    let h_at_mw = mw * ih / iw;
    if h_at_mw <= mh {
        (mw as u32, h_at_mw as u32)
    } else {
        let w_at_mh = mh * iw / ih;
        (w_at_mh as u32, mh as u32)
    }
}

fn fit_image_for_block(
    src: &str,
    alt: &str,
    iw: u32,
    ih: u32,
    max_w: u32,
    max_h: u32,
    slide_h: u32,
) -> (u32, u32) {
    let Some(math_meta) = crate::math::math_image_meta(src, alt) else {
        return fit_image(iw, ih, max_w, max_h);
    };
    let natural_w = ((iw.max(1) as u64 * 12_700) / 2).min(u32::MAX as u64) as u32;
    let natural_h = ((ih.max(1) as u64 * 12_700) / 2).min(u32::MAX as u64) as u32;
    let configured_max_h = math_meta
        .max_height_px
        .map(|px| u32::from(px).saturating_mul(12_700));
    let math_max_h = configured_max_h
        .unwrap_or(slide_h * 28 / 100)
        .min(max_h)
        .max(1)
        .min(natural_h.max(1));
    fit_image(
        natural_w,
        natural_h,
        max_w.min(natural_w.max(1)),
        math_max_h,
    )
}

fn image_x_for_block(src: &str, alt: &str, x: u32, w: u32, display_w: u32) -> u32 {
    match crate::math::math_image_meta(src, alt).map(|meta| meta.align) {
        Some(crate::math::MathBlockAlign::Left) => x,
        Some(crate::math::MathBlockAlign::Right) => x + w.saturating_sub(display_w),
        Some(crate::math::MathBlockAlign::Center) | None => x + w.saturating_sub(display_w) / 2,
    }
}

fn progress_width(width: u32, num: usize, total: usize) -> u32 {
    if width == 0 || total == 0 {
        return 0;
    }
    ((width as u64 * num.max(1) as u64) / total as u64)
        .min(width as u64)
        .max(1) as u32
}

fn letterspaced(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        for u in c.to_uppercase() {
            out.push(u);
        }
    }
    out
}

fn author_date(author: Option<&str>, date: Option<&str>) -> Option<String> {
    let mut s = String::new();
    if let Some(a) = author {
        s.push_str(a);
    }
    if author.is_some() && date.is_some() {
        s.push_str("  ·  ");
    }
    if let Some(d) = date {
        s.push_str(d);
    }
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cff_outline_detection_picks_the_right_font_program() {
        // CFF-flavoured OpenType (the `OTTO` magic) must take the
        // CIDFontType0 / FontFile3 path; TrueType-flavoured sfnts the
        // CIDFontType2 / FontFile2 path.
        assert!(font_has_cff_outlines(b"OTTO\x00\x04\x00\x80"));
        assert!(!font_has_cff_outlines(&[0x00, 0x01, 0x00, 0x00])); // TrueType
        assert!(!font_has_cff_outlines(b"true")); // legacy TrueType magic
                                                  // Every bundled DejaVu face is TrueType (glyf) — the default output
                                                  // must stay on the FontFile2 path.
        for face in crate::font::FONTS {
            assert!(
                !font_has_cff_outlines(face),
                "bundled faces are expected to be TrueType",
            );
        }
    }
}
