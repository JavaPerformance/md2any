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
//! When the user passes `--cjk PATH`, the named file is loaded into an
//! additional slot ([`CJK_FACE`]) and treated as a per-character fallback
//! for codepoints DejaVu can't render. The same subsetting pipeline runs
//! over it, so even a 20 MB Noto CJK source contributes only the glyphs
//! the deck actually uses.

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

/// Container holding every face the PDF writer needs, including the
/// optional runtime-loaded CJK fallback. The bundled DejaVu faces are
/// always present; `cjk` is `Some` only when the user passed `--cjk PATH`.
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
        if let Some(path) = cjk_path {
            let raw = std::fs::read(path)
                .map_err(|e| anyhow::anyhow!("read CJK font {}: {}", path.display(), e))?;
            bytes.push(raw);
            names.push("CJKFallback".to_string());
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

    pub fn face_count(&self) -> usize {
        self.metrics.len()
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
