//! SVG and PNG slide-image export.
//!
//! The writer emits one file per slide. SVG is produced directly from the
//! paginated IR; PNG is the same SVG rasterised through the existing optional
//! `svg` feature pipeline.

use crate::image;
use crate::ir::{runs_text, Block, ListItem, Run, Slide, SlideKind};
use crate::layout::Layout;
use crate::syntax::{self, TokenKind};
use crate::theme::Theme;
use anyhow::Result;
use std::fmt::Write as _;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct SlideFile {
    pub name: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Svg,
    Png,
}

pub fn write_files(
    slides: &[Slide],
    theme: &Theme,
    layout: &Layout,
    deck_title: &str,
    author: &str,
    base_dir: &Path,
    logo: Option<&Path>,
    direction: Option<&str>,
    format: ImageFormat,
) -> Result<Vec<SlideFile>> {
    let svg_files = write_svg_files(
        slides, theme, layout, deck_title, author, base_dir, logo, direction,
    )?;
    if format == ImageFormat::Svg {
        return Ok(svg_files);
    }

    let mut png_files = Vec::with_capacity(svg_files.len());
    for file in svg_files {
        let origin = file.name.clone();
        let png = image::rasterize_svg_to_png(&file.bytes, &origin)?;
        png_files.push(SlideFile {
            name: origin.replace(".svg", ".png"),
            bytes: png.bytes,
        });
    }
    Ok(png_files)
}

pub fn write_svg_files(
    slides: &[Slide],
    theme: &Theme,
    layout: &Layout,
    deck_title: &str,
    author: &str,
    base_dir: &Path,
    logo: Option<&Path>,
    direction: Option<&str>,
) -> Result<Vec<SlideFile>> {
    let mut files = Vec::with_capacity(slides.len());
    let total = slides.len();
    let logo_uri = logo.map(|path| image_data_uri(base_dir, path.to_string_lossy().as_ref()));
    for (idx, slide) in slides.iter().enumerate() {
        let svg = render_slide(
            slide,
            idx,
            total,
            theme,
            layout,
            deck_title,
            author,
            base_dir,
            logo_uri.as_deref(),
            direction,
        )?;
        files.push(SlideFile {
            name: format!("slide-{:03}.svg", idx + 1),
            bytes: svg.into_bytes(),
        });
    }
    Ok(files)
}

fn render_slide(
    slide: &Slide,
    idx: usize,
    total: usize,
    theme: &Theme,
    layout: &Layout,
    deck_title: &str,
    author: &str,
    base_dir: &Path,
    logo_uri: Option<&str>,
    direction: Option<&str>,
) -> Result<String> {
    let rtl = matches!(direction, Some("rtl"));
    let w = emu_to_px(theme.slide_w);
    let h = emu_to_px(theme.slide_h);
    let mut out = String::new();
    write!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{:.0}" height="{:.0}" viewBox="0 0 {:.2} {:.2}" role="img" aria-label="Slide {} of {}">"#,
        w,
        h,
        w,
        h,
        idx + 1,
        total
    )?;
    write!(
        out,
        "<title>{}</title><desc>{}</desc>",
        escape_xml(&slide.title),
        escape_xml(&format!(
            "{} slide {} of {}",
            if deck_title.is_empty() {
                "md2any"
            } else {
                deck_title
            },
            idx + 1,
            total
        ))
    )?;
    write!(
        out,
        r##"<rect x="0" y="0" width="{:.2}" height="{:.2}" fill="#{}"/>"##,
        w, h, theme.bg
    )?;
    if let Some(bg) = &slide.bg_image {
        let uri = escape_attr(&image_data_uri(base_dir, bg));
        write!(
            out,
            r##"<image href="{uri}" xlink:href="{uri}" x="0" y="0" width="{:.2}" height="{:.2}" preserveAspectRatio="xMidYMid slice"/><rect x="0" y="0" width="{:.2}" height="{:.2}" fill="#{}" opacity="0.78"/>"##,
            w, h, w, h, theme.bg
        )?;
    }

    if let Some((src, _, _)) = slide.full_page_image() {
        let uri = escape_attr(&image_data_uri(base_dir, src));
        write!(
            out,
            r#"<image href="{uri}" xlink:href="{uri}" x="0" y="0" width="{:.2}" height="{:.2}" preserveAspectRatio="xMidYMid meet"/>"#,
            w, h
        )?;
        out.push_str("</svg>\n");
        return Ok(out);
    }
    if let Some((lines, lang)) = slide.full_page_code() {
        render_full_page_code(&mut out, lines, lang, theme, w, h)?;
        out.push_str("</svg>\n");
        return Ok(out);
    }

    render_layout_chrome(&mut out, theme, layout, deck_title, idx, total, w, h, rtl)?;

    match &slide.kind {
        SlideKind::Title {
            subtitle,
            author: slide_author,
            date,
        } => render_title_slide(
            &mut out,
            slide,
            theme,
            subtitle,
            slide_author,
            date,
            w,
            h,
            rtl,
        )?,
        SlideKind::Section => render_section_slide(&mut out, slide, theme, layout, w, h, rtl)?,
        SlideKind::Content => render_content_slide(
            &mut out, slide, idx, total, theme, layout, base_dir, logo_uri, author, w, h, rtl,
        )?,
    }

    out.push_str("</svg>\n");
    Ok(out)
}

fn render_layout_chrome(
    out: &mut String,
    theme: &Theme,
    layout: &Layout,
    deck_title: &str,
    idx: usize,
    total: usize,
    w: f32,
    h: f32,
    rtl: bool,
) -> Result<()> {
    if layout.shows_rail() {
        let x = if rtl { w - w * 0.012 } else { 0.0 };
        write!(
            out,
            r##"<rect x="{:.2}" y="0" width="{:.2}" height="{:.2}" fill="#{}"/>"##,
            x,
            w * 0.012,
            h,
            theme.accent
        )?;
    }
    if layout.shows_sidebar() {
        let sw = w * 0.18;
        let x = if rtl { w - sw } else { 0.0 };
        write!(
            out,
            r##"<rect x="{:.2}" y="0" width="{:.2}" height="{:.2}" fill="#{}"/>"##,
            x, sw, h, theme.accent
        )?;
        let text_x = if rtl { x + sw - 18.0 } else { x + 18.0 };
        let anchor = if rtl { "end" } else { "start" };
        draw_wrapped_text(
            out,
            deck_title,
            TextBox {
                x: text_x,
                y: 36.0,
                width: sw - 36.0,
                font_size: 11.0,
                line_height: 14.0,
                fill: &theme.on_accent,
                font: &theme.title_font,
                weight: "700",
                style: "normal",
                anchor,
            },
        )?;
        write!(
            out,
            r##"<text x="{:.2}" y="{:.2}" font-family="{}" font-size="11" font-weight="700" fill="#{}" text-anchor="{}">{:02}/{:02}</text>"##,
            text_x,
            h - 30.0,
            escape_attr(svg_font_family(&theme.title_font)),
            theme.on_accent,
            anchor,
            idx + 1,
            total
        )?;
    }
    Ok(())
}

fn render_title_slide(
    out: &mut String,
    slide: &Slide,
    theme: &Theme,
    subtitle: &Option<String>,
    author: &Option<String>,
    date: &Option<String>,
    w: f32,
    h: f32,
    rtl: bool,
) -> Result<()> {
    let x = if rtl { w - 78.0 } else { 78.0 };
    let anchor = if rtl { "end" } else { "start" };
    let mut y = h * 0.36;
    let title_size = centipt_to_pt(theme.hero_size).min(58.0).max(30.0);
    y = draw_wrapped_text(
        out,
        &slide.title,
        TextBox {
            x,
            y,
            width: w * 0.78,
            font_size: title_size,
            line_height: title_size * 1.08,
            fill: &theme.title_color,
            font: &theme.title_font,
            weight: "700",
            style: "normal",
            anchor,
        },
    )? + 14.0;
    if let Some(subtitle) = subtitle {
        y = draw_wrapped_text(
            out,
            subtitle,
            TextBox {
                x,
                y,
                width: w * 0.7,
                font_size: 23.0,
                line_height: 29.0,
                fill: &theme.body_color,
                font: &theme.body_font,
                weight: "400",
                style: "normal",
                anchor,
            },
        )? + 18.0;
    }
    let mut byline = String::new();
    if let Some(author) = author {
        byline.push_str(author);
    }
    if author.is_some() && date.is_some() {
        byline.push_str(" - ");
    }
    if let Some(date) = date {
        byline.push_str(date);
    }
    if !byline.is_empty() {
        let _ = draw_wrapped_text(
            out,
            &byline,
            TextBox {
                x,
                y,
                width: w * 0.7,
                font_size: 14.0,
                line_height: 18.0,
                fill: &theme.muted_color,
                font: &theme.body_font,
                weight: "400",
                style: "normal",
                anchor,
            },
        )?;
    }
    Ok(())
}

fn render_section_slide(
    out: &mut String,
    slide: &Slide,
    theme: &Theme,
    layout: &Layout,
    w: f32,
    h: f32,
    rtl: bool,
) -> Result<()> {
    if layout.section_full_bg() {
        write!(
            out,
            r##"<rect x="0" y="0" width="{:.2}" height="{:.2}" fill="#{}"/>"##,
            w, h, theme.section_bg
        )?;
    }
    let fill = if layout.section_full_bg() {
        &theme.section_text
    } else {
        &theme.title_color
    };
    let x = if rtl { w - 78.0 } else { 78.0 };
    let anchor = if rtl { "end" } else { "start" };
    let size = centipt_to_pt(theme.hero_size).min(54.0).max(30.0);
    let _ = draw_wrapped_text(
        out,
        &slide.title,
        TextBox {
            x,
            y: h * 0.46,
            width: w * 0.78,
            font_size: size,
            line_height: size * 1.08,
            fill,
            font: &theme.title_font,
            weight: "700",
            style: "normal",
            anchor,
        },
    )?;
    Ok(())
}

fn render_content_slide(
    out: &mut String,
    slide: &Slide,
    idx: usize,
    total: usize,
    theme: &Theme,
    layout: &Layout,
    base_dir: &Path,
    logo_uri: Option<&str>,
    _author: &str,
    w: f32,
    h: f32,
    rtl: bool,
) -> Result<()> {
    let margin = 54.0;
    let sidebar = if layout.shows_sidebar() {
        w * 0.18
    } else {
        0.0
    };
    let x = if rtl {
        margin
    } else {
        margin + sidebar.max(if layout.shows_rail() { w * 0.022 } else { 0.0 })
    };
    let content_w = w - x - margin - if rtl { sidebar } else { 0.0 };
    let title_size = centipt_to_pt(theme.title_size).min(34.0).max(21.0);
    let mut y = margin;

    if layout.title_block_bg() {
        write!(
            out,
            r##"<rect x="0" y="0" width="{:.2}" height="{:.2}" fill="#{}"/>"##,
            w,
            margin + title_size * 1.55,
            theme.accent
        )?;
    }
    let title_fill = if layout.title_block_bg() {
        &theme.on_accent
    } else {
        &theme.title_color
    };
    let (title_x, anchor) = if rtl {
        (w - x, "end")
    } else if theme.title_center {
        (x + content_w / 2.0, "middle")
    } else {
        (x, "start")
    };
    y = draw_wrapped_text(
        out,
        &slide.title,
        TextBox {
            x: title_x,
            y,
            width: content_w,
            font_size: title_size,
            line_height: title_size * 1.12,
            fill: title_fill,
            font: &theme.title_font,
            weight: "700",
            style: "normal",
            anchor,
        },
    )? + 12.0;
    if layout.title_underline() {
        let progress_w = progress_width(content_w, idx + 1, total);
        let progress_x = if rtl { x + content_w - progress_w } else { x };
        write!(
            out,
            r##"<rect x="{:.2}" y="{:.2}" width="{:.2}" height="1.0" fill="#{}"/><rect x="{:.2}" y="{:.2}" width="{:.2}" height="2.5" fill="#{}"/>"##,
            x,
            y + 0.75,
            content_w,
            theme.divider,
            progress_x,
            y,
            progress_w,
            theme.accent
        )?;
        y += 18.0;
    }

    let mut ctx = RenderCtx {
        out,
        theme,
        base_dir,
        rtl,
        valign: slide.valign(),
        col_frac: slide.col_frac().unwrap_or(0.5),
        text_align: slide.text_align(),
    };
    let _ = render_blocks(&mut ctx, &slide.blocks, x, y, content_w, h - margin * 1.6)?;
    if let Some(logo_uri) = logo_uri {
        let uri = escape_attr(logo_uri);
        write!(
            ctx.out,
            r#"<image href="{uri}" xlink:href="{uri}" x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" preserveAspectRatio="xMaxYMax meet"/>"#,
            if rtl { margin } else { w - margin - 58.0 },
            h - margin,
            58.0,
            34.0
        )?;
    }
    Ok(())
}

struct RenderCtx<'a> {
    out: &'a mut String,
    theme: &'a Theme,
    base_dir: &'a Path,
    rtl: bool,
    /// Vertical alignment for two-column rows: "top", "center", or "bottom".
    valign: &'a str,
    /// Left-column fraction (0..1) for two-column rows; 0.5 is an even split.
    col_frac: f32,
    /// Horizontal text alignment: "left", "center", or "right".
    text_align: &'a str,
}

fn render_blocks(
    ctx: &mut RenderCtx<'_>,
    blocks: &[Block],
    x: f32,
    mut y: f32,
    width: f32,
    bottom: f32,
) -> Result<f32> {
    for block in blocks {
        if y > bottom {
            break;
        }
        y = render_block(ctx, block, x, y, width, bottom)? + 6.0;
    }
    Ok(y)
}

fn render_block(
    ctx: &mut RenderCtx<'_>,
    block: &Block,
    x: f32,
    y: f32,
    width: f32,
    bottom: f32,
) -> Result<f32> {
    let theme = ctx.theme;
    let anchor = match ctx.text_align {
        "center" => "middle",
        "right" if ctx.rtl => "start",
        "right" => "end",
        _ if ctx.rtl => "end",
        _ => "start",
    };
    let tx = match ctx.text_align {
        "center" => x + width / 2.0,
        "right" if ctx.rtl => x,
        "right" => x + width,
        _ if ctx.rtl => x + width,
        _ => x,
    };
    let body_size = centipt_to_pt(theme.body_size);
    match block {
        Block::Paragraph(runs) => draw_runs_plain(
            ctx.out,
            runs,
            TextBox {
                x: tx,
                y,
                width,
                font_size: body_size,
                line_height: body_size * 1.28,
                fill: &theme.body_color,
                font: &theme.body_font,
                weight: "400",
                style: "normal",
                anchor,
            },
        ),
        Block::Heading { level, runs } => {
            let size = (body_size + (7.0 - f32::from(*level)).max(0.0) * 2.0).max(body_size);
            draw_runs_plain(
                ctx.out,
                runs,
                TextBox {
                    x: tx,
                    y: y + 4.0,
                    width,
                    font_size: size,
                    line_height: size * 1.2,
                    fill: &theme.title_color,
                    font: &theme.title_font,
                    weight: "700",
                    style: "normal",
                    anchor,
                },
            )
        }
        Block::List(items) => render_list(ctx, items, x, y, width),
        Block::CodeBlock {
            lang,
            title,
            lines,
            line_numbers,
            start_line,
            ..
        } => render_code_block(
            ctx,
            lang.as_deref(),
            title.as_deref(),
            lines,
            *line_numbers,
            *start_line,
            x,
            y,
            width,
        ),
        Block::Quote(paragraphs) => {
            let mut yy = y + 4.0;
            let rule_x = if ctx.rtl { x + width - 4.0 } else { x };
            write!(
                ctx.out,
                r##"<rect x="{:.2}" y="{:.2}" width="3" height="{:.2}" fill="#{}"/>"##,
                rule_x,
                y,
                (paragraphs.len().max(1) as f32) * body_size * 1.5,
                theme.accent
            )?;
            for runs in paragraphs {
                yy = draw_runs_plain(
                    ctx.out,
                    runs,
                    TextBox {
                        x: if ctx.rtl { x + width - 16.0 } else { x + 16.0 },
                        y: yy,
                        width: width - 24.0,
                        font_size: body_size,
                        line_height: body_size * 1.28,
                        fill: &theme.title_color,
                        font: &theme.body_font,
                        weight: "400",
                        style: "normal",
                        anchor,
                    },
                )? + 3.0;
            }
            Ok(yy)
        }
        Block::Callout { kind, body } => {
            let color = callout_color(kind);
            let color_hex = format!("#{color}");
            let (_icon, label) = callout_label(kind);
            let padx = 14.0;
            let text_x = x + padx;
            let text_w = width - 2.0 * padx;
            let mark = ctx.out.len();
            // Title line (in the callout colour), then the body paragraphs.
            let mut yy = draw_runs_plain(
                ctx.out,
                &[Run::plain(label)],
                TextBox {
                    x: text_x,
                    y: y + padx + body_size,
                    width: text_w,
                    font_size: body_size * 0.92,
                    line_height: body_size * 1.2,
                    fill: &color_hex,
                    font: &theme.body_font,
                    weight: "700",
                    style: "normal",
                    anchor: "start",
                },
            )? + 4.0;
            for runs in body {
                yy = draw_runs_plain(
                    ctx.out,
                    runs,
                    TextBox {
                        x: text_x,
                        y: yy,
                        width: text_w,
                        font_size: body_size,
                        line_height: body_size * 1.28,
                        fill: &theme.title_color,
                        font: &theme.body_font,
                        weight: "400",
                        style: "normal",
                        anchor: "start",
                    },
                )? + 3.0;
            }
            let box_h = (yy - y) + padx * 0.4;
            // Insert the tinted background + colour bar *before* the text.
            let rect = format!(
                r##"<rect x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" rx="6" fill="{color_hex}" fill-opacity="0.13"/><rect x="{:.2}" y="{:.2}" width="4" height="{:.2}" rx="2" fill="{color_hex}"/>"##,
                x, y, width, box_h, x, y, box_h
            );
            ctx.out.insert_str(mark, &rect);
            Ok(y + box_h + 6.0)
        }
        Block::Table {
            headers,
            rows,
            aligns,
        } => render_table(ctx, headers, rows, aligns, x, y, width),
        Block::ColumnBreak => Ok(y),
        Block::Columns { left, right } => {
            let gap = 22.0;
            let avail_w = width - gap;
            let lcw = avail_w * ctx.col_frac;
            let rcw = avail_w - lcw;
            let right_x = x + lcw + gap;
            let factor = match ctx.valign {
                "center" => 0.5,
                "bottom" => 1.0,
                _ => 0.0,
            };
            if factor == 0.0 {
                let left_y = render_blocks(ctx, left, x, y, lcw, bottom)?;
                let right_y = render_blocks(ctx, right, right_x, y, rcw, bottom)?;
                return Ok(left_y.max(right_y));
            }
            // Measure each column's height by rendering then rewinding the buffer,
            // then place it within the available height so the shorter column
            // (usually the text beside a tall image) is centred/bottom-aligned.
            // Using the available height (not the taller column) guarantees the
            // offset can never push content past `bottom`.
            let avail = (bottom - y).max(0.0);
            let mark = ctx.out.len();
            let lh = (render_blocks(ctx, left, x, y, lcw, bottom)? - y).max(0.0);
            ctx.out.truncate(mark);
            let rh = (render_blocks(ctx, right, right_x, y, rcw, bottom)? - y).max(0.0);
            ctx.out.truncate(mark);
            let ly = y + (avail - lh).max(0.0) * factor;
            let ry = y + (avail - rh).max(0.0) * factor;
            let left_y = render_blocks(ctx, left, x, ly, lcw, bottom)?;
            let right_y = render_blocks(ctx, right, right_x, ry, rcw, bottom)?;
            Ok(left_y.max(right_y))
        }
        Block::Image {
            src,
            alt,
            width_pct,
        } => render_image(ctx, src, alt, *width_pct, x, y, width, bottom),
        Block::Footnotes(items) => {
            let foot_y = bottom - (items.len().max(1) as f32 * 11.0);
            render_list(ctx, items, x, foot_y, width)
        }
    }
}

fn draw_runs_plain(out: &mut String, runs: &[Run], tb: TextBox<'_>) -> Result<f32> {
    draw_wrapped_text(out, &runs_text(runs), tb)
}

/// Accent colour (hex, no `#`) for a callout kind.
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

fn render_list(
    ctx: &mut RenderCtx<'_>,
    items: &[ListItem],
    x: f32,
    mut y: f32,
    width: f32,
) -> Result<f32> {
    let theme = ctx.theme;
    let body_size = centipt_to_pt(theme.body_size);
    let mut counters = vec![0usize; 10];
    for item in items {
        let level = usize::from(item.level).min(9);
        if item.ordered {
            counters[level] += 1;
        }
        for counter in counters.iter_mut().skip(level + 1) {
            *counter = 0;
        }
        let indent = level as f32 * 19.0;
        // Task-list items carry their own ☐/☑ marker in the runs, so suppress
        // the bullet and start the text where the bullet would have gone.
        let is_task = item.is_task();
        let marker = if item.ordered {
            format!("{}.", counters[level].max(1))
        } else {
            "*".to_string()
        };
        let marker_x = if ctx.rtl {
            x + width - indent
        } else {
            x + indent
        };
        let text_x = if is_task {
            marker_x
        } else if ctx.rtl {
            marker_x - 20.0
        } else {
            marker_x + 20.0
        };
        let anchor = if ctx.rtl { "end" } else { "start" };
        if !is_task {
            write!(
                ctx.out,
                r##"<text x="{:.2}" y="{:.2}" font-family="{}" font-size="{:.2}" font-weight="700" fill="#{}" text-anchor="{}">{}</text>"##,
                marker_x,
                y,
                escape_attr(svg_font_family(&theme.body_font)),
                body_size,
                theme.accent,
                anchor,
                escape_xml(&marker)
            )?;
        }
        y = draw_runs_plain(
            ctx.out,
            &item.runs,
            TextBox {
                x: text_x,
                y,
                width: width - indent - 24.0,
                font_size: body_size,
                line_height: body_size * 1.25,
                fill: &theme.body_color,
                font: &theme.body_font,
                weight: "400",
                style: "normal",
                anchor,
            },
        )? + 4.0;
    }
    Ok(y)
}

fn render_code_block(
    ctx: &mut RenderCtx<'_>,
    lang: Option<&str>,
    title: Option<&str>,
    lines: &[String],
    line_numbers: bool,
    start_line: usize,
    x: f32,
    y: f32,
    width: f32,
) -> Result<f32> {
    let theme = ctx.theme;
    let fs = centipt_to_pt(theme.code_size);
    let lh = fs * 1.34;
    let caption_h = if title.is_some() || lang.is_some() {
        fs * 1.75
    } else {
        0.0
    };
    let height = caption_h + lines.len().max(1) as f32 * lh + 16.0;
    write!(
        ctx.out,
        r##"<rect x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" rx="4" fill="#{}" stroke="#{}" stroke-width="1"/>"##,
        x, y, width, height, theme.code_bg, theme.divider
    )?;
    if title.is_some() || lang.is_some() {
        let caption = match (title, lang) {
            (Some(title), Some(lang)) => format!("{title}  {lang}"),
            (Some(title), None) => title.to_string(),
            (None, Some(lang)) => lang.to_string(),
            (None, None) => String::new(),
        };
        write!(
            ctx.out,
            r##"<text x="{:.2}" y="{:.2}" font-family="{}" font-size="{:.2}" font-weight="700" fill="#{}">{}</text><line x1="{:.2}" y1="{:.2}" x2="{:.2}" y2="{:.2}" stroke="#{}" stroke-width="1"/>"##,
            x + 10.0,
            y + fs * 1.15,
            escape_attr(svg_font_family(&theme.body_font)),
            fs * 0.82,
            theme.muted_color,
            escape_xml(&caption),
            x,
            y + caption_h,
            x + width,
            y + caption_h,
            theme.divider
        )?;
    }
    let mut yy = y + caption_h + fs + 7.0;
    let gutter = if line_numbers { fs * 3.2 } else { 0.0 };
    let token_lines = syntax::tokenize(lines, lang);
    for (idx, tokens) in token_lines.iter().enumerate() {
        if line_numbers {
            write!(
                ctx.out,
                r##"<text x="{:.2}" y="{:.2}" font-family="{}" font-size="{:.2}" fill="#{}" text-anchor="end">{}</text>"##,
                x + gutter,
                yy,
                escape_attr(svg_font_family(&theme.mono_font)),
                fs,
                theme.muted_color,
                start_line + idx
            )?;
        }
        let mut xx = x + gutter + 10.0;
        write!(
            ctx.out,
            r##"<text x="{:.2}" y="{:.2}" font-family="{}" font-size="{:.2}" fill="#{}">"##,
            xx,
            yy,
            escape_attr(svg_font_family(&theme.mono_font)),
            fs,
            theme.code_text
        )?;
        for token in tokens {
            let style = theme.syntax_style(token.kind);
            write!(
                ctx.out,
                r##"<tspan fill="#{}"{}{}>{}</tspan>"##,
                style.color,
                if style.bold {
                    r#" font-weight="700""#
                } else {
                    ""
                },
                if style.italic {
                    r#" font-style="italic""#
                } else {
                    ""
                },
                escape_xml(&token.text)
            )?;
            xx += token.text.chars().count() as f32 * fs * 0.58;
            if xx > x + width - 12.0 {
                break;
            }
        }
        ctx.out.push_str("</text>");
        yy += lh;
    }
    Ok(y + height)
}

fn render_full_page_code(
    out: &mut String,
    lines: &[String],
    lang: Option<&str>,
    theme: &Theme,
    w: f32,
    h: f32,
) -> Result<()> {
    let math_markup = crate::math::is_markup_text_language(lang);
    if math_markup {
        render_full_page_math_markup(out, lines, theme, w, h)?;
        return Ok(());
    }

    let rendered_lines = crate::math::translate_markup_lines(lines, lang);
    let margin = if h > w { 22.0 } else { 28.0 };
    let base = centipt_to_pt(theme.code_size).min(if h > w { 8.5 } else { 9.5 });
    let max_chars = rendered_lines
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(1)
        .max(1) as f32;
    let line_count = rendered_lines.len().max(1) as f32;
    let line_h_factor = if math_markup { 1.10_f32 } else { 1.18_f32 };
    let char_factor = if math_markup { 0.50_f32 } else { 0.58_f32 };
    let scale_w = (w - 2.0 * margin).max(1.0) / (max_chars * base * char_factor).max(1.0);
    let scale_h = (h - 2.0 * margin).max(1.0) / (line_count * base * line_h_factor).max(1.0);
    let fs = (base * scale_w.min(scale_h).min(1.0)).max(4.5);
    let lh = fs * line_h_factor;
    let total_h = lh * line_count;
    let mut y = margin + ((h - 2.0 * margin) - total_h).max(0.0) / 2.0 + fs;
    let font = if math_markup {
        escape_attr(svg_font_family(&theme.body_font))
    } else {
        escape_attr(svg_font_family(&theme.mono_font))
    };
    let x = if math_markup { w / 2.0 } else { margin };
    let anchor = if math_markup { "middle" } else { "start" };
    let style = if math_markup {
        r#" font-style="italic""#
    } else {
        ""
    };
    for line in &rendered_lines {
        write!(
            out,
            r##"<text x="{:.2}" y="{:.2}" font-family="{}" font-size="{:.2}" fill="#{}" text-anchor="{}"{}>{}</text>"##,
            x,
            y,
            font,
            fs,
            theme.title_color,
            anchor,
            style,
            escape_xml(line)
        )?;
        y += lh;
    }
    Ok(())
}

fn render_full_page_math_markup(
    out: &mut String,
    lines: &[String],
    theme: &Theme,
    w: f32,
    h: f32,
) -> Result<()> {
    let margin_x = if h > w { 18.0 } else { 25.0 };
    let margin_y = 20.5;
    let max_w = (w - margin_x * 2.0).max(1.0);
    let max_h = (h - margin_y * 2.0).max(1.0);
    let base_size = 28.0_f32;
    let gap = base_size * 0.24;
    let raw_layouts = svg_math_line_layouts(lines);
    let layouts = fit_svg_math_line_layouts(lines, raw_layouts, base_size, gap, max_w / max_h);
    let (max_line_w, total_h) = svg_math_metrics(&layouts, base_size, gap);
    let scale = (max_w / max_line_w)
        .min(max_h / total_h.max(1.0))
        .min(1.0)
        .max(0.045);
    let rendered_h = total_h * scale;
    let mut top_y = margin_y + (max_h - rendered_h).max(0.0) / 2.0;
    let font = escape_attr(svg_font_family(&theme.body_font));

    for layout in &layouts {
        if let Some(layout) = layout {
            let line_w = layout.width * scale;
            let x = margin_x + (max_w - line_w).max(0.0) / 2.0;
            draw_svg_math_layout(out, layout, x, top_y, scale, theme, &font)?;
            top_y += layout.height * scale + gap * scale;
        } else {
            top_y += base_size * 0.65 * scale + gap * scale;
        }
    }
    Ok(())
}

fn draw_svg_math_layout(
    out: &mut String,
    layout: &crate::math::MathTextLayout,
    origin_x: f32,
    top_y: f32,
    scale: f32,
    theme: &Theme,
    font: &str,
) -> Result<()> {
    for draw in &layout.draws {
        match draw {
            crate::math::MathLayoutDraw::Text {
                x,
                y,
                size,
                text,
                bold,
            } => {
                if text.trim().is_empty() {
                    continue;
                }
                let weight = if *bold { r#" font-weight="bold""# } else { "" };
                write!(
                    out,
                    r##"<text x="{:.2}" y="{:.2}" font-family="{}" font-size="{:.2}" font-style="italic"{weight} fill="#{}">{}</text>"##,
                    origin_x + x * scale,
                    top_y + y * scale,
                    font,
                    (size * scale).max(1.0),
                    theme.title_color,
                    escape_xml(text)
                )?;
            }
            crate::math::MathLayoutDraw::Line {
                x1,
                y1,
                x2,
                y2,
                stroke_width,
            } => {
                write!(
                    out,
                    r##"<line x1="{:.2}" y1="{:.2}" x2="{:.2}" y2="{:.2}" stroke="#{}" stroke-width="{:.2}" stroke-linecap="round"/>"##,
                    origin_x + x1 * scale,
                    top_y + y1 * scale,
                    origin_x + x2 * scale,
                    top_y + y2 * scale,
                    theme.title_color,
                    (stroke_width * scale).max(0.22)
                )?;
            }
            crate::math::MathLayoutDraw::Polyline {
                points,
                stroke_width,
            } => {
                let points = points
                    .iter()
                    .map(|(x, y)| format!("{:.2},{:.2}", origin_x + x * scale, top_y + y * scale))
                    .collect::<Vec<_>>()
                    .join(" ");
                write!(
                    out,
                    r##"<polyline points="{}" fill="none" stroke="#{}" stroke-width="{:.2}" stroke-linecap="round" stroke-linejoin="round"/>"##,
                    points,
                    theme.title_color,
                    (stroke_width * scale).max(0.22)
                )?;
            }
            crate::math::MathLayoutDraw::Delimiter {
                x,
                y,
                height,
                token,
                ..
            } => {
                write!(
                    out,
                    r##"<text x="{:.2}" y="{:.2}" font-family="{}" font-size="{:.2}" font-style="italic" fill="#{}">{}</text>"##,
                    origin_x + x * scale,
                    top_y + (y + height * 0.82) * scale,
                    font,
                    (height * scale).max(1.0),
                    theme.title_color,
                    escape_xml(token)
                )?;
            }
        }
    }
    Ok(())
}

fn svg_math_line_layouts(lines: &[String]) -> Vec<Option<crate::math::MathTextLayout>> {
    lines
        .iter()
        .map(|line| {
            if line.trim().is_empty() {
                None
            } else {
                Some(crate::math::layout_markup_text(line, 100))
            }
        })
        .collect()
}

fn pack_svg_math_line_layouts(
    lines: &[String],
    target_width: f32,
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
            current_layout = Some(crate::math::layout_markup_text(&current, 100));
            continue;
        }

        let candidate = format!("{current} {trimmed}");
        let candidate_layout = crate::math::layout_markup_text(&candidate, 100);
        if candidate_layout.width <= target_width {
            current = candidate;
            current_layout = Some(candidate_layout);
        } else {
            if let Some(layout) = current_layout.take() {
                out.push(Some(layout));
            }
            current.clear();
            current.push_str(trimmed);
            current_layout = Some(crate::math::layout_markup_text(&current, 100));
        }
    }

    if let Some(layout) = current_layout {
        out.push(Some(layout));
    }
    out
}

fn fit_svg_math_line_layouts(
    lines: &[String],
    raw_layouts: Vec<Option<crate::math::MathTextLayout>>,
    base_size: f32,
    gap: f32,
    page_ratio: f32,
) -> Vec<Option<crate::math::MathTextLayout>> {
    let (raw_max_line_w, raw_total_h) = svg_math_metrics(&raw_layouts, base_size, gap);
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
        let packed = pack_svg_math_line_layouts(lines, target);
        let (packed_w, packed_h) = svg_math_metrics(&packed, base_size, gap);
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

fn svg_math_metrics(
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

fn render_table(
    ctx: &mut RenderCtx<'_>,
    headers: &[Vec<Run>],
    rows: &[Vec<Vec<Run>>],
    aligns: &[crate::ir::ColumnAlign],
    x: f32,
    y: f32,
    width: f32,
) -> Result<f32> {
    let theme = ctx.theme;
    let body_size = centipt_to_pt(theme.body_size);
    let cols = headers
        .len()
        .max(rows.iter().map(Vec::len).max().unwrap_or(1))
        .max(1);
    let col_w = width / cols as f32;
    let row_h = body_size * 1.65;
    // (text-anchor, x) for a column's cell text, honouring GFM alignment.
    let cell_anchor = |idx: usize| -> (&'static str, f32) {
        let left = x + idx as f32 * col_w;
        match aligns.get(idx) {
            Some(crate::ir::ColumnAlign::Center) => ("middle", left + col_w / 2.0),
            Some(crate::ir::ColumnAlign::Right) => ("end", left + col_w - 6.0),
            _ => ("start", left + 6.0),
        }
    };
    let mut yy = y;
    write!(
        ctx.out,
        r##"<rect x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" fill="#{}" stroke="#{}" stroke-width="1"/>"##,
        x, yy, width, row_h, theme.accent_soft, theme.divider
    )?;
    for (idx, cell) in headers.iter().enumerate() {
        let (anchor, cx) = cell_anchor(idx);
        draw_runs_plain(
            ctx.out,
            cell,
            TextBox {
                x: cx,
                y: yy + row_h * 0.64,
                width: col_w - 12.0,
                font_size: body_size * 0.72,
                line_height: body_size,
                fill: &theme.title_color,
                font: &theme.body_font,
                weight: "700",
                style: "normal",
                anchor,
            },
        )?;
    }
    yy += row_h;
    let table_band_bg = theme.table_band_bg();
    for (row_idx, row) in rows.iter().enumerate() {
        let bg = if row_idx % 2 == 1 {
            &table_band_bg
        } else {
            &theme.bg
        };
        write!(
            ctx.out,
            r##"<rect x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" fill="#{}" stroke="#{}" stroke-width="1"/>"##,
            x, yy, width, row_h, bg, theme.divider
        )?;
        for (idx, cell) in row.iter().enumerate() {
            let (anchor, cx) = cell_anchor(idx);
            draw_runs_plain(
                ctx.out,
                cell,
                TextBox {
                    x: cx,
                    y: yy + row_h * 0.64,
                    width: col_w - 12.0,
                    font_size: body_size * 0.68,
                    line_height: body_size,
                    fill: &theme.body_color,
                    font: &theme.body_font,
                    weight: "400",
                    style: "normal",
                    anchor,
                },
            )?;
        }
        yy += row_h;
    }
    Ok(yy)
}

fn render_image(
    ctx: &mut RenderCtx<'_>,
    src: &str,
    alt: &str,
    width_pct: Option<u8>,
    x: f32,
    y: f32,
    width: f32,
    bottom: f32,
) -> Result<f32> {
    let meta = image::load_any_or_placeholder(ctx.base_dir, src);
    let max_w = width * (width_pct.unwrap_or(100).clamp(1, 100) as f32 / 100.0);
    let max_h = (bottom - y).max(80.0).min(260.0);
    let math_meta = crate::math::math_image_meta(src, alt);
    let (img_w, img_h) = if let Some(math_meta) = math_meta {
        let natural_w = (meta.width.max(1) as f32) / 2.0;
        let natural_h = (meta.height.max(1) as f32) / 2.0;
        let math_max_h = math_meta
            .max_height_px
            .map(f32::from)
            .unwrap_or(160.0)
            .min(max_h);
        let scale = (max_w.min(natural_w) / natural_w)
            .min(math_max_h.min(natural_h) / natural_h)
            .min(1.0);
        (natural_w * scale, natural_h * scale)
    } else {
        let scale = (max_w / meta.width.max(1) as f32).min(max_h / meta.height.max(1) as f32);
        (meta.width as f32 * scale, meta.height as f32 * scale)
    };
    let img_x = match math_meta.map(|meta| meta.align) {
        Some(crate::math::MathBlockAlign::Left) => x,
        Some(crate::math::MathBlockAlign::Right) => x + width - img_w,
        Some(crate::math::MathBlockAlign::Center) => x + (width - img_w) / 2.0,
        None if ctx.rtl => x + width - img_w,
        None => x,
    };
    if math_meta.is_some() {
        if let Some(svg) = inline_generated_math_svg(src, img_x, y, img_w, img_h) {
            ctx.out.push_str(&svg);
            return Ok(y + img_h + 8.0);
        }
    }
    let uri = escape_attr(&data_uri(&meta));
    write!(
        ctx.out,
        r#"<image href="{uri}" xlink:href="{uri}" x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" preserveAspectRatio="xMidYMid meet"/>"#,
        img_x, y, img_w, img_h
    )?;
    Ok(y + img_h + 8.0)
}

fn inline_generated_math_svg(src: &str, x: f32, y: f32, width: f32, height: f32) -> Option<String> {
    let svg = crate::math::decode_generated_math_svg(src)?;
    let open_end = svg.find('>')?;
    let close_start = svg.rfind("</svg>")?;
    let root = &svg[..=open_end];
    let view_box = xml_attr_value(root, "viewBox")?;
    let body = &svg[open_end + 1..close_start];
    Some(format!(
        r#"<svg x="{x:.2}" y="{y:.2}" width="{width:.2}" height="{height:.2}" viewBox="{}" overflow="visible">{body}</svg>"#,
        escape_attr(&view_box)
    ))
}

fn xml_attr_value(tag: &str, attr: &str) -> Option<String> {
    let needle = format!(r#"{attr}=""#);
    let start = tag.find(&needle)? + needle.len();
    let rest = &tag[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

#[derive(Clone, Copy)]
struct TextBox<'a> {
    x: f32,
    y: f32,
    width: f32,
    font_size: f32,
    line_height: f32,
    fill: &'a str,
    font: &'a str,
    weight: &'a str,
    style: &'a str,
    anchor: &'a str,
}

fn draw_wrapped_text(out: &mut String, text: &str, tb: TextBox<'_>) -> Result<f32> {
    let lines = wrap_text(text, tb.width, tb.font_size);
    let mut y = tb.y;
    for line in lines {
        write!(
            out,
            r##"<text x="{:.2}" y="{:.2}" font-family="{}" font-size="{:.2}" font-weight="{}" font-style="{}" fill="#{}" text-anchor="{}">{}</text>"##,
            tb.x,
            y,
            escape_attr(svg_font_family(tb.font)),
            tb.font_size,
            tb.weight,
            tb.style,
            tb.fill,
            tb.anchor,
            escape_xml(&line)
        )?;
        y += tb.line_height;
    }
    Ok(y)
}

fn wrap_text(text: &str, width: f32, font_size: f32) -> Vec<String> {
    let max_chars = ((width / (font_size * 0.54)).floor() as usize).clamp(8, 140);
    let mut lines = Vec::new();
    for raw_line in text.lines() {
        let mut current = String::new();
        for word in raw_line.split_whitespace() {
            // Hard-break a word longer than a whole line so it wraps instead of
            // overflowing the slide edge.
            if word.chars().count() > max_chars {
                if !current.is_empty() {
                    lines.push(std::mem::take(&mut current));
                }
                let chars: Vec<char> = word.chars().collect();
                let mut i = 0;
                while i < chars.len() {
                    let end = (i + max_chars).min(chars.len());
                    let chunk: String = chars[i..end].iter().collect();
                    if end < chars.len() {
                        lines.push(chunk);
                    } else {
                        current = chunk;
                    }
                    i = end;
                }
                continue;
            }
            let next_len = current.chars().count()
                + if current.is_empty() { 0 } else { 1 }
                + word.chars().count();
            if next_len > max_chars && !current.is_empty() {
                lines.push(current);
                current = String::new();
            }
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        }
        if current.is_empty() {
            lines.push(String::new());
        } else {
            lines.push(current);
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn image_data_uri(base_dir: &Path, src: &str) -> String {
    let meta = image::load_any_or_placeholder(base_dir, src);
    data_uri(&meta)
}

fn data_uri(meta: &image::ImageMeta) -> String {
    let mime = match meta.ext {
        "jpeg" | "jpg" => "image/jpeg",
        "png" => "image/png",
        _ => "application/octet-stream",
    };
    format!("data:{mime};base64,{}", base64(&meta.bytes))
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(b2 & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn emu_to_px(emu: u32) -> f32 {
    emu as f32 / 12_700.0
}

fn centipt_to_pt(size: u32) -> f32 {
    size as f32 / 100.0
}

fn progress_width(width: f32, num: usize, total: usize) -> f32 {
    if width <= 0.0 || total == 0 {
        return 0.0;
    }
    (width * num.max(1) as f32 / total as f32)
        .min(width)
        .max(0.5)
}

fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

fn escape_attr(s: &str) -> String {
    escape_xml(s)
}

fn svg_font_family(font: &str) -> &'static str {
    let lower = font.to_ascii_lowercase();
    if lower.contains("mono")
        || lower.contains("consolas")
        || lower.contains("courier")
        || lower.contains("code")
    {
        "DejaVu Sans Mono"
    } else {
        "DejaVu Sans"
    }
}

#[allow(dead_code)]
fn token_class(kind: TokenKind) -> &'static str {
    match kind {
        TokenKind::Default => "tok-default",
        TokenKind::Keyword => "tok-keyword",
        TokenKind::String => "tok-string",
        TokenKind::Number => "tok-number",
        TokenKind::Comment => "tok-comment",
        TokenKind::Function => "tok-function",
        TokenKind::Type => "tok-type",
        TokenKind::Attribute => "tok-attribute",
    }
}
