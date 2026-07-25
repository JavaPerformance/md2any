//! Embedded TrueType fonts used for PDF output.
//!
//! md2any ships DejaVu Sans (regular + bold + oblique + bold-oblique) and
//! DejaVu Sans Mono in the binary so PDF output can render Unicode glyphs
//! that the PDF standard-14 fonts (Helvetica + Courier family with WinAnsi
//! encoding) cannot — Greek letters, math operators, sub/superscripts, and
//! anything else outside Latin-1.
//!
//! The font files are read at compile time via `include_bytes!`. At PDF
//! emit time each face is wrapped in a Type0 / CIDFontType2 dictionary,
//! subset to just the glyphs the deck actually references, and embedded
//! with Identity-H encoding (text is written as 16-bit big-endian glyph
//! IDs). Subsetting typically turns the ~3 MB of bundled font binaries
//! into a few tens of KB per PDF.
//!
//! Runtime font paths can replace the PDF sans/mono faces and add
//! per-character fallback faces for codepoints the primary font cannot
//! render. The same subsetting pipeline runs over every loaded font, so
//! even a 20 MB Noto CJK source contributes only the glyphs the deck
//! actually uses.

use crate::ir::{self, Block, Slide, SlideKind};
use std::collections::BTreeMap;
use std::path::Path;
use ttf_parser::{Face, GlyphId};

/// Distinct font faces md2any can ask the PDF writer to use. The index
/// values are the slot positions in [`FONTS`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaceKind {
    SansRegular,
    SansBold,
    SansOblique,
    SansBoldOblique,
    Mono,
}

impl FaceKind {
    pub fn index(self) -> usize {
        match self {
            FaceKind::SansRegular => 0,
            FaceKind::SansBold => 1,
            FaceKind::SansOblique => 2,
            FaceKind::SansBoldOblique => 3,
            FaceKind::Mono => 4,
        }
    }

    /// PostScript name written into the PDF font dictionary.
    pub fn ps_name(self) -> &'static str {
        match self {
            FaceKind::SansRegular => "DejaVuSans",
            FaceKind::SansBold => "DejaVuSans-Bold",
            FaceKind::SansOblique => "DejaVuSans-Oblique",
            FaceKind::SansBoldOblique => "DejaVuSans-BoldOblique",
            FaceKind::Mono => "DejaVuSansMono",
        }
    }
}

pub const FACE_COUNT: usize = 5;

/// Slot index of the optional runtime-loaded CJK fallback face. If a CJK
/// font is loaded, it gets this slot; otherwise the slot is empty.
pub const CJK_FACE: usize = FACE_COUNT;

/// Raw TTF bytes of each bundled face, in [`FaceKind::index`] order. Static
/// lifetime so the rest of the code can hand out `&'static [u8]` slices.
pub static FONTS: [&[u8]; FACE_COUNT] = [
    include_bytes!("../assets/fonts/DejaVuSans.ttf"),
    include_bytes!("../assets/fonts/DejaVuSans-Bold.ttf"),
    include_bytes!("../assets/fonts/DejaVuSans-Oblique.ttf"),
    include_bytes!("../assets/fonts/DejaVuSans-BoldOblique.ttf"),
    include_bytes!("../assets/fonts/DejaVuSansMono.ttf"),
];

#[derive(Debug, Clone, Default)]
pub struct PdfFontOptions<'a> {
    /// Optional replacement for the PDF sans family. The same face is used
    /// for regular/bold/italic slots; this keeps the interface small while
    /// still allowing brand fonts and broad Unicode fonts.
    pub pdf_font: Option<&'a Path>,
    /// Optional replacement for the PDF monospace/code face.
    pub pdf_mono_font: Option<&'a Path>,
    /// Per-character fallback fonts tried after the primary face.
    pub fallback_fonts: Vec<&'a Path>,
}

/// Container holding every face the PDF writer needs, including the
/// optional runtime-loaded fallback fonts. The bundled DejaVu faces are
/// always present unless replaced by [`PdfFontOptions`].
pub struct PdfFonts {
    /// One entry per slot, length = [`FACE_COUNT`] + (1 if CJK loaded).
    pub metrics: Vec<FaceMetrics>,
    /// Raw TTF bytes for each face — bundled ones reference the static
    /// `&'static [u8]` from [`FONTS`]; the CJK face owns a `Vec<u8>` from
    /// disk.
    pub bytes: Vec<Vec<u8>>,
    /// PostScript name per face, used in the PDF font dictionaries.
    pub names: Vec<String>,
}

impl PdfFonts {
    pub fn load(cjk_path: Option<&std::path::Path>) -> anyhow::Result<Self> {
        let mut options = PdfFontOptions::default();
        if let Some(path) = cjk_path {
            options.fallback_fonts.push(path);
        }
        Self::load_with_options(options)
    }

    pub fn load_with_options(options: PdfFontOptions<'_>) -> anyhow::Result<Self> {
        let mut bytes: Vec<Vec<u8>> = FONTS.iter().map(|b| b.to_vec()).collect();
        let mut names: Vec<String> = (0..FACE_COUNT)
            .map(|i| {
                let kind = match i {
                    0 => FaceKind::SansRegular,
                    1 => FaceKind::SansBold,
                    2 => FaceKind::SansOblique,
                    3 => FaceKind::SansBoldOblique,
                    _ => FaceKind::Mono,
                };
                kind.ps_name().to_string()
            })
            .collect();

        if let Some(path) = options.pdf_font {
            let raw = read_font(path, "PDF font")?;
            for (idx, name) in [
                (0usize, "CustomSans"),
                (1usize, "CustomSans-Bold"),
                (2usize, "CustomSans-Oblique"),
                (3usize, "CustomSans-BoldOblique"),
            ] {
                bytes[idx] = raw.clone();
                names[idx] = name.to_string();
            }
        }
        if let Some(path) = options.pdf_mono_font {
            bytes[FaceKind::Mono.index()] = read_font(path, "PDF mono font")?;
            names[FaceKind::Mono.index()] = "CustomMono".to_string();
        }
        for (idx, path) in options.fallback_fonts.iter().enumerate() {
            bytes.push(read_font(path, "PDF fallback font")?);
            names.push(format!("FontFallback{}", idx + 1));
        }

        let mut metrics = Vec::with_capacity(bytes.len());
        for b in &bytes {
            metrics.push(FaceMetrics::parse(b)?);
        }
        Ok(PdfFonts {
            metrics,
            bytes,
            names,
        })
    }

    pub fn has_cjk(&self) -> bool {
        self.metrics.len() > FACE_COUNT
    }

    pub fn has_fallbacks(&self) -> bool {
        self.metrics.len() > FACE_COUNT
    }

    pub fn face_count(&self) -> usize {
        self.metrics.len()
    }

    /// Choose a face and glyph for one character. Returns `None` when
    /// neither the primary face nor any configured fallback can render it.
    pub fn face_for_char(&self, primary: usize, c: char) -> Option<(usize, u16)> {
        if primary < self.metrics.len() {
            let face = &self.metrics[primary];
            if let Some(gid) = face.glyph_for_char(&self.bytes[primary], c) {
                return Some((primary, gid));
            }
        }
        for idx in FACE_COUNT..self.metrics.len() {
            let face = &self.metrics[idx];
            if let Some(gid) = face.glyph_for_char(&self.bytes[idx], c) {
                return Some((idx, gid));
            }
        }
        None
    }
}

fn read_font(path: &Path, label: &str) -> anyhow::Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| anyhow::anyhow!("read {label} {}: {}", path.display(), e))
}

/// Locate a system math font for full-page markup math (SM Lagrangian, etc.).
///
/// Order: `$MD2ANY_MATH_FONT`, then well-known STIX / TeX Gyre / Cambria paths.
/// Returns `None` on wasm or when nothing is installed — callers fall back to
/// DejaVu metrics / the bundled sans face.
pub fn find_system_math_font() -> Option<std::path::PathBuf> {
    #[cfg(target_arch = "wasm32")]
    {
        return None;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Ok(p) = std::env::var("MD2ANY_MATH_FONT") {
            let path = std::path::PathBuf::from(p);
            if path.is_file() {
                return Some(path);
            }
        }
        const CANDIDATES: &[&str] = &[
            // Linux packages (stix-fonts, fonts-stix, texlive, …)
            "/usr/share/fonts/stix-fonts/STIXTwoMath-Regular.otf",
            "/usr/share/fonts/STIX/STIXTwoMath-Regular.otf",
            "/usr/share/fonts/opentype/stix/STIXTwoMath-Regular.otf",
            "/usr/share/fonts/opentype/stix-math/STIXTwoMath-Regular.otf",
            "/usr/share/fonts/truetype/stix/STIXTwoMath-Regular.otf",
            "/usr/share/fonts/truetype/dejavu/DejaVuMathTeXGyre.ttf",
            "/usr/share/fonts/opentype/tex-gyre-math/texgyredejavu-math.otf",
            "/usr/share/fonts/opentype/texgyre/texgyredejavu-math.otf",
            // macOS
            "/Library/Fonts/STIXTwoMath-Regular.otf",
            "/System/Library/Fonts/Supplemental/STIXTwoMath.otf",
            // Windows
            "C:\\Windows\\Fonts\\STIXTwoMath-Regular.otf",
            "C:\\Windows\\Fonts\\cambria.ttc",
        ];
        for c in CANDIDATES {
            let p = std::path::Path::new(c);
            if p.is_file() {
                return Some(p.to_path_buf());
            }
        }
        None
    }
}

#[derive(Debug, Clone)]
pub struct FontAudit {
    pub slide_count: usize,
    pub face_names: Vec<String>,
    pub fallback_hits: Vec<GlyphAuditHit>,
    pub missing: Vec<GlyphAuditHit>,
}

#[derive(Debug, Clone)]
pub struct GlyphAuditHit {
    pub ch: char,
    pub codepoint: String,
    pub count: usize,
    pub first_slide: usize,
    pub context: String,
    pub primary: &'static str,
    pub face: Option<String>,
}

#[derive(Debug, Clone)]
struct GlyphAccumulator {
    count: usize,
    first_slide: usize,
    context: String,
    primary: &'static str,
    face: Option<String>,
}

pub fn audit_pdf_fonts(slides: &[Slide], fonts: &PdfFonts) -> FontAudit {
    let mut audit = AuditBuilder {
        fonts,
        fallback_hits: BTreeMap::new(),
        missing: BTreeMap::new(),
    };
    for (idx, slide) in slides.iter().enumerate() {
        let num = idx + 1;
        audit.text(
            num,
            "slide title",
            &slide.title,
            FaceKind::SansRegular.index(),
        );
        match &slide.kind {
            SlideKind::Title {
                subtitle,
                author,
                date,
            } => {
                if let Some(text) = subtitle {
                    audit.text(num, "title subtitle", text, FaceKind::SansRegular.index());
                }
                if let Some(text) = author {
                    audit.text(num, "title author", text, FaceKind::SansRegular.index());
                }
                if let Some(text) = date {
                    audit.text(num, "title date", text, FaceKind::SansRegular.index());
                }
            }
            SlideKind::Section | SlideKind::Content => {}
        }
        if let Some(notes) = &slide.notes {
            audit.text(num, "speaker notes", notes, FaceKind::SansRegular.index());
        }
        audit.blocks(num, &slide.blocks);
    }
    audit.finish(slides.len())
}

struct AuditBuilder<'a> {
    fonts: &'a PdfFonts,
    fallback_hits: BTreeMap<(char, &'static str, usize), GlyphAccumulator>,
    missing: BTreeMap<(char, &'static str), GlyphAccumulator>,
}

impl AuditBuilder<'_> {
    fn blocks(&mut self, slide: usize, blocks: &[Block]) {
        for block in blocks {
            match block {
                Block::Paragraph(runs) | Block::Heading { runs, .. } => {
                    self.runs(slide, "text", runs, FaceKind::SansRegular.index());
                }
                Block::List(items) | Block::Footnotes(items) => {
                    for item in items {
                        self.runs(
                            slide,
                            "list item",
                            &item.runs,
                            FaceKind::SansRegular.index(),
                        );
                    }
                }
                Block::CodeBlock { title, lines, .. } => {
                    if let Some(title) = title {
                        self.text(slide, "code caption", title, FaceKind::Mono.index());
                    }
                    for line in lines {
                        self.text(slide, "code", line, FaceKind::Mono.index());
                    }
                }
                Block::Quote(paragraphs)
                | Block::Callout {
                    body: paragraphs, ..
                } => {
                    for runs in paragraphs {
                        self.runs(slide, "quote", runs, FaceKind::SansRegular.index());
                    }
                }
                Block::Table { headers, rows, .. } => {
                    for cell in headers.iter().chain(rows.iter().flatten()) {
                        self.runs(slide, "table cell", cell, FaceKind::SansRegular.index());
                    }
                }
                Block::Columns { left, right } => {
                    self.blocks(slide, left);
                    self.blocks(slide, right);
                }
                Block::Image { alt, .. } => {
                    self.text(slide, "image alt", alt, FaceKind::SansRegular.index());
                }
                Block::Cards { cards, .. } => {
                    self.blocks(slide, &crate::ir::cards_as_blocks(cards))
                }
                Block::ColumnBreak => {}
            }
        }
    }

    fn runs(&mut self, slide: usize, context: &str, runs: &[ir::Run], primary: usize) {
        for run in runs {
            self.text(slide, context, &run.text, primary);
        }
    }

    fn text(&mut self, slide: usize, context: &str, text: &str, primary: usize) {
        let primary_name = if primary == FaceKind::Mono.index() {
            "mono"
        } else {
            "sans"
        };
        for ch in text.chars() {
            if ch.is_control() {
                continue;
            }
            match self.fonts.face_for_char(primary, ch) {
                Some((face_idx, _)) if face_idx >= FACE_COUNT => {
                    let key = (ch, primary_name, face_idx);
                    let face = self.fonts.names.get(face_idx).cloned();
                    record_hit(
                        &mut self.fallback_hits,
                        key,
                        ch,
                        slide,
                        context,
                        primary_name,
                        face,
                    );
                }
                Some(_) => {}
                None => {
                    let key = (ch, primary_name);
                    record_hit(
                        &mut self.missing,
                        key,
                        ch,
                        slide,
                        context,
                        primary_name,
                        None,
                    );
                }
            }
        }
    }

    fn finish(self, slide_count: usize) -> FontAudit {
        FontAudit {
            slide_count,
            face_names: self.fonts.names.clone(),
            fallback_hits: self
                .fallback_hits
                .into_iter()
                .map(|((ch, _, _), acc)| acc.into_hit(ch))
                .collect(),
            missing: self
                .missing
                .into_iter()
                .map(|((ch, _), acc)| acc.into_hit(ch))
                .collect(),
        }
    }
}

fn record_hit<K: Ord>(
    map: &mut BTreeMap<K, GlyphAccumulator>,
    key: K,
    ch: char,
    slide: usize,
    context: &str,
    primary: &'static str,
    face: Option<String>,
) {
    let entry = map.entry(key).or_insert_with(|| GlyphAccumulator {
        count: 0,
        first_slide: slide,
        context: context.to_string(),
        primary,
        face,
    });
    let _ = ch;
    entry.count += 1;
}

impl GlyphAccumulator {
    fn into_hit(self, ch: char) -> GlyphAuditHit {
        GlyphAuditHit {
            ch,
            codepoint: format!("U+{:04X}", ch as u32),
            count: self.count,
            first_slide: self.first_slide,
            context: self.context,
            primary: self.primary,
            face: self.face,
        }
    }
}

/// Compact, owned snapshot of the font metrics PDF needs. Built once at
/// the start of a render so we don't reparse the TTF on every glyph lookup.
pub struct FaceMetrics {
    pub units_per_em: u16,
    pub ascent: i16,
    pub descent: i16,
    pub bbox: (i16, i16, i16, i16),
    pub italic_angle: f32,
    pub cap_height: i16,
    pub num_glyphs: u16,
    /// Advance width per glyph ID, in font units.
    pub widths: Vec<u16>,
    /// Cached Unicode → glyph ID lookup. Built lazily by [`glyph_for_char`]
    /// so we don't iterate the cmap exhaustively at startup.
    cmap_cache: std::sync::Mutex<std::collections::HashMap<u32, u16>>,
}

impl FaceMetrics {
    pub fn parse(ttf: &[u8]) -> anyhow::Result<Self> {
        let face = Face::parse(ttf, 0).map_err(|e| anyhow::anyhow!("parse TTF: {:?}", e))?;
        let units_per_em = face.units_per_em();
        let ascent = face.ascender();
        let descent = face.descender();
        let bbox = face.global_bounding_box();
        let bbox = (bbox.x_min, bbox.y_min, bbox.x_max, bbox.y_max);
        let italic_angle = face.italic_angle().unwrap_or(0.0);
        let cap_height = face.capital_height().unwrap_or(ascent);
        let num_glyphs = face.number_of_glyphs();

        let mut widths = vec![0u16; num_glyphs as usize];
        for gid in 0..num_glyphs {
            widths[gid as usize] = face.glyph_hor_advance(GlyphId(gid)).unwrap_or(0);
        }

        Ok(FaceMetrics {
            units_per_em,
            ascent,
            descent,
            bbox,
            italic_angle,
            cap_height,
            num_glyphs,
            widths,
            cmap_cache: std::sync::Mutex::new(std::collections::HashMap::new()),
        })
    }

    /// Map a Unicode codepoint to a glyph ID for this face. Returns
    /// `None` if the font has no glyph for that codepoint; the caller
    /// should substitute glyph 0 (.notdef) in that case.
    pub fn glyph_for_char(&self, ttf: &[u8], c: char) -> Option<u16> {
        let cp = c as u32;
        let mut cache = self.cmap_cache.lock().unwrap();
        if let Some(&gid) = cache.get(&cp) {
            return if gid == u16::MAX { None } else { Some(gid) };
        }
        // Reparse the face to access cmap. ttf-parser doesn't keep the
        // cmap available without the Face, but parsing is fast.
        let face = Face::parse(ttf, 0).ok()?;
        let gid = face.glyph_index(c).map(|g| g.0);
        cache.insert(cp, gid.unwrap_or(u16::MAX));
        gid
    }

    /// Width of a glyph in font units. Out-of-range glyph IDs report 0.
    pub fn glyph_width(&self, gid: u16) -> u16 {
        *self.widths.get(gid as usize).unwrap_or(&0)
    }

    /// Build the (glyph-id → first Unicode codepoint) mapping needed for a
    /// PDF `/ToUnicode` CMap stream. We sweep the Basic Multilingual Plane
    /// + the Supplementary Multilingual Plane (mathematical glyphs live up
    /// at U+1D400+) and ask the font for the glyph each codepoint resolves
    /// to. Ambiguous reverse mappings (two codepoints sharing a glyph) keep
    /// the *first* codepoint — that's also what PDF readers do when copying
    /// text, so search and clipboard get the canonical character.
    pub fn cid_to_unicode(&self, ttf: &[u8]) -> Vec<(u16, char)> {
        let Some(face) = Face::parse(ttf, 0).ok() else {
            return Vec::new();
        };
        let mut by_gid: std::collections::BTreeMap<u16, char> = std::collections::BTreeMap::new();
        for cp in (0x21u32..=0xFFFFu32).chain(0x1D400u32..=0x1D7FFu32) {
            let Some(c) = char::from_u32(cp) else {
                continue;
            };
            if let Some(gid) = face.glyph_index(c) {
                if gid.0 == 0 {
                    continue;
                }
                by_gid.entry(gid.0).or_insert(c);
            }
        }
        by_gid.into_iter().collect()
    }

    /// Convert font-unit width to typographic points at the given size.
    pub fn glyph_width_pt(&self, gid: u16, size_pt: f32) -> f32 {
        (self.glyph_width(gid) as f32 / self.units_per_em as f32) * size_pt
    }
}
