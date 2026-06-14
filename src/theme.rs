//! Theme definitions — colour palettes, fonts, slide dimensions.
//!
//! A gallery of built-in themes ([`THEME_NAMES`] — `light`, `dark`, plus
//! `corporate`, `sepia`, `contrast`, `midnight`, `terminal`, `pastel`, each a
//! palette layered on a light/dark base), aspect-ratio presets plus a
//! free-form `WxH[unit]` parser, and an optional YAML overlay
//! ([`ThemeOverride`]) for brand-specific palettes. The renderer never
//! reads colours directly — it always goes through the [`Theme`] struct
//! that this module produces, so swapping themes is a one-line change.
//!
//! Internally we store all dimensions in EMU (English Metric Units, 914,400
//! per inch) since that's the PPTX / DOCX unit and the renderer wants to
//! convert exactly once.

use crate::syntax::TokenKind;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Deserializer};
use std::path::Path;

/// Slide renderers cap each image at this fraction of the slide height so
/// the title bar and any surrounding text stay visible. Centralised here
/// because three renderers (PDF / PPTX / ODP) **and** the pagination
/// weight in [`crate::paginate`] all need the same value — keeping it in
/// one place stops the four sites drifting apart, which previously caused
/// text-after-image slides to overflow the page footer.
pub const IMAGE_MAX_HEIGHT_FRACTION_NUM: u32 = 65;
pub const IMAGE_MAX_HEIGHT_FRACTION_DEN: u32 = 100;

/// Lists with more than this many items auto-flow into a two-column
/// layout (on landscape slides) or stay single-column (on portrait).
/// Centralised because paginate, pptx and odp all need to agree —
/// otherwise the wrap point differs per output format.
pub const LONG_LIST_THRESHOLD: usize = 12;

/// User-supplied YAML overlay (only keys present are applied). Hex colors
/// accept `#RRGGBB` or `RRGGBB`; the leading `#` is stripped.
#[derive(Debug, Default, Deserialize)]
pub struct ThemeOverride {
    pub bg: Option<String>,
    pub title_color: Option<String>,
    pub body_color: Option<String>,
    pub muted_color: Option<String>,
    pub accent: Option<String>,
    pub accent_soft: Option<String>,
    pub divider: Option<String>,
    pub code_bg: Option<String>,
    pub code_text: Option<String>,
    pub code_accent: Option<String>,
    pub section_bg: Option<String>,
    pub section_text: Option<String>,
    pub link: Option<String>,
    pub on_accent: Option<String>,

    /// Slide-title alignment: `left` (default) or `center`.
    pub title_align: Option<String>,

    pub title_font: Option<String>,
    pub body_font: Option<String>,
    pub mono_font: Option<String>,
    pub pdf_font: Option<String>,
    pub pdf_mono_font: Option<String>,
    #[serde(default, deserialize_with = "deserialize_font_fallback")]
    pub font_fallback: Vec<String>,

    pub title_size: Option<u32>,
    pub body_size: Option<u32>,
    pub code_size: Option<u32>,
    pub hero_size: Option<u32>,

    pub syntax: Option<SyntaxOverride>,
}

#[derive(Debug, Default, Deserialize)]
pub struct SyntaxOverride {
    pub keyword: Option<String>,
    pub string: Option<String>,
    pub number: Option<String>,
    pub comment: Option<String>,
    pub function: Option<String>,
    #[serde(rename = "type")]
    pub type_color: Option<String>,
    pub attribute: Option<String>,
}

/// Validate and normalise a theme-file colour value. Accepts `#RRGGBB`,
/// `RRGGBB`, `#RGB`, or `RGB` (the short form is expanded to 6 hex
/// digits). Anything else is rejected so we never feed corrupt strings
/// into the XML emitters downstream.
fn normalize_hex(s: &str, field: &str) -> Result<String> {
    let trimmed = s.trim().trim_start_matches('#');
    if !trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!(
            "{}: {:?} is not a valid hex colour (use #RRGGBB or #RGB)",
            field,
            s
        );
    }
    let expanded = match trimmed.len() {
        6 => trimmed.to_string(),
        3 => trimmed.chars().flat_map(|c| [c, c]).collect(),
        _ => bail!(
            "{}: {:?} must have 3 or 6 hex digits (got {})",
            field,
            s,
            trimmed.len()
        ),
    };
    Ok(expanded.to_ascii_uppercase())
}

fn hex_luma(s: &str) -> Option<f32> {
    let trimmed = s.trim().trim_start_matches('#');
    if trimmed.len() != 6 || !trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&trimmed[0..2], 16).ok()? as f32 / 255.0;
    let g = u8::from_str_radix(&trimmed[2..4], 16).ok()? as f32 / 255.0;
    let b = u8::from_str_radix(&trimmed[4..6], 16).ok()? as f32 / 255.0;
    Some(0.2126 * r + 0.7152 * g + 0.0722 * b)
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum FontFallbackYaml {
    One(String),
    Many(Vec<String>),
}

fn deserialize_font_fallback<'de, D>(deserializer: D) -> std::result::Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let Some(value) = Option::<FontFallbackYaml>::deserialize(deserializer)? else {
        return Ok(Vec::new());
    };
    let out = match value {
        FontFallbackYaml::One(value) => split_font_fallbacks(&value),
        FontFallbackYaml::Many(values) => values
            .into_iter()
            .flat_map(|value| split_font_fallbacks(&value))
            .collect(),
    };
    Ok(out)
}

pub fn split_font_fallbacks(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

#[derive(Debug, Clone)]
pub struct SyntaxStyle {
    pub color: String,
    pub italic: bool,
    pub bold: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeTheme {
    Dark,
    Light,
    Match,
}

impl Default for CodeTheme {
    fn default() -> Self {
        CodeTheme::Dark
    }
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub name: &'static str,
    pub slide_w: u32,
    pub slide_h: u32,
    pub aspect_kind: &'static str,
    pub portrait: bool,
    pub title_size: u32,
    pub body_size: u32,
    pub code_size: u32,
    pub hero_size: u32,

    pub bg: String,
    pub title_color: String,
    pub body_color: String,
    pub muted_color: String,
    pub accent: String,
    pub accent_soft: String,
    pub divider: String,
    pub code_bg: String,
    pub code_text: String,
    pub code_accent: String,
    pub section_bg: String,
    pub section_text: String,
    pub link: String,
    pub on_accent: String,
    /// Centre `##` slide titles instead of left-aligning them.
    pub title_center: bool,

    pub title_font: String,
    pub body_font: String,
    pub mono_font: String,
    pub pdf_font: Option<String>,
    pub pdf_mono_font: Option<String>,
    pub font_fallback: Vec<String>,

    pub syntax_keyword: String,
    pub syntax_string: String,
    pub syntax_number: String,
    pub syntax_comment: String,
    pub syntax_function: String,
    pub syntax_type: String,
    pub syntax_attribute: String,
}

impl Theme {
    pub fn table_band_bg(&self) -> String {
        if hex_luma(&self.bg).unwrap_or(if self.name == "dark" { 0.0 } else { 1.0 }) > 0.45 {
            "F8FAFC".into()
        } else {
            "111827".into()
        }
    }

    pub fn syntax_style(&self, kind: TokenKind) -> SyntaxStyle {
        match kind {
            TokenKind::Keyword => SyntaxStyle {
                color: self.syntax_keyword.clone(),
                italic: false,
                bold: true,
            },
            TokenKind::String => SyntaxStyle {
                color: self.syntax_string.clone(),
                italic: false,
                bold: false,
            },
            TokenKind::Number => SyntaxStyle {
                color: self.syntax_number.clone(),
                italic: false,
                bold: false,
            },
            TokenKind::Comment => SyntaxStyle {
                color: self.syntax_comment.clone(),
                italic: true,
                bold: false,
            },
            TokenKind::Function => SyntaxStyle {
                color: self.syntax_function.clone(),
                italic: false,
                bold: false,
            },
            TokenKind::Type => SyntaxStyle {
                color: self.syntax_type.clone(),
                italic: false,
                bold: false,
            },
            TokenKind::Attribute => SyntaxStyle {
                color: self.syntax_attribute.clone(),
                italic: false,
                bold: false,
            },
            TokenKind::Default => SyntaxStyle {
                color: self.code_text.clone(),
                italic: false,
                bold: false,
            },
        }
    }
}

/// All selectable built-in theme names (light/dark plus the gallery), used in
/// error messages and the `--theme-gallery` preview.
pub const THEME_NAMES: [&str; 8] = [
    "light",
    "dark",
    "corporate",
    "sepia",
    "contrast",
    "midnight",
    "terminal",
    "pastel",
];

/// Build a [`ThemeOverride`] from a set of `field: "hex/name"` pairs.
macro_rules! palette {
    ($($field:ident: $value:expr),* $(,)?) => {
        ThemeOverride { $($field: Some($value.into()),)* ..Default::default() }
    };
}

/// A built-in gallery theme: `(dark_base, static_name, palette)`. The palette
/// is layered onto the light/dark base, so geometry and sizing come for free.
fn builtin_theme(name: &str) -> Option<(bool, &'static str, ThemeOverride)> {
    let theme = match name {
        "corporate" => (
            false,
            "corporate",
            palette! {
                title_color: "0B1F44", body_color: "334155", accent: "1E3A8A",
                accent_soft: "DBEAFE", divider: "CBD5E1", section_bg: "0B1F44",
                section_text: "F8FAFC", link: "1E3A8A", title_align: "center",
                title_font: "Georgia", body_font: "Calibri",
            },
        ),
        "sepia" => (
            false,
            "sepia",
            palette! {
                bg: "FBF3E3", title_color: "4A3728", body_color: "5B4636",
                muted_color: "A1846B", accent: "B45309", accent_soft: "FDE9C8",
                divider: "E7D6B8", section_bg: "4A3728", section_text: "FBF3E3",
                link: "B45309", code_bg: "F3E6CE", code_text: "4A3728",
                title_font: "Georgia", body_font: "Georgia",
            },
        ),
        "contrast" => (
            false,
            "contrast",
            palette! {
                bg: "FFFFFF", title_color: "000000", body_color: "111111",
                muted_color: "444444", accent: "D90429", accent_soft: "FFE5E9",
                divider: "000000", section_bg: "000000", section_text: "FFFF00",
                link: "D90429", on_accent: "FFFFFF",
            },
        ),
        "midnight" => (
            true,
            "midnight",
            palette! {
                bg: "0A0F2C", title_color: "E0E7FF", body_color: "C7D2FE",
                muted_color: "6366F1", accent: "818CF8", accent_soft: "312E81",
                divider: "1E1B4B", section_bg: "312E81", section_text: "EEF2FF",
                link: "A5B4FC", on_accent: "0A0F2C", code_bg: "11163A",
            },
        ),
        "terminal" => (
            true,
            "terminal",
            palette! {
                bg: "0A0A0A", title_color: "39FF14", body_color: "B6FFB0",
                muted_color: "4E8E45", accent: "00FF66", accent_soft: "0F2A0F",
                divider: "1A2A1A", section_bg: "0A1A0A", section_text: "39FF14",
                link: "00FF66", on_accent: "0A0A0A", code_bg: "0F1A0F",
                code_text: "B6FFB0", title_font: "DejaVu Sans Mono",
                body_font: "DejaVu Sans Mono", mono_font: "DejaVu Sans Mono",
            },
        ),
        "pastel" => (
            false,
            "pastel",
            palette! {
                bg: "FFF8FB", title_color: "5B2A86", body_color: "6D597A",
                muted_color: "B8A7C9", accent: "FF6B9D", accent_soft: "FFE0EC",
                divider: "F3D9E5", section_bg: "B5838D", section_text: "FFFFFF",
                link: "FF6B9D", code_bg: "F7ECF2",
            },
        ),
        _ => return None,
    };
    Some(theme)
}

impl Theme {
    pub fn resolve(name: &str, aspect: &str, font: Option<&str>) -> Result<Self> {
        let lower = aspect.trim().to_ascii_lowercase();
        let (slide_w, slide_h, aspect_kind, portrait) = match lower.as_str() {
            "16:9" | "16x9" | "widescreen" => (12192000, 6858000, "screen16x9", false),
            "4:3" | "4x3" | "standard" => (9144000, 6858000, "screen4x3", false),
            "9:16" | "9x16" | "portrait" => (6858000, 12192000, "custom", true),
            "a4" => (7560000, 10692000, "custom", true),
            "a4-landscape" | "a4l" => (10692000, 7560000, "custom", false),
            "a3" => (10692000, 15120000, "custom", true),
            "a5" => (5328000, 7560000, "custom", true),
            "letter" => (7772400, 10058400, "custom", true),
            "letter-landscape" | "letter-l" => (10058400, 7772400, "custom", false),
            "legal" => (7772400, 12801600, "custom", true),
            "tabloid" => (10058400, 15544800, "custom", true),
            other => parse_custom_aspect(other)?,
        };

        let (title_size, body_size, code_size, hero_size) = if portrait {
            (2400, 1700, 1400, 4200)
        } else {
            (2800, 1800, 1500, 5400)
        };

        let body_font = font
            .map(|s| s.to_string())
            .unwrap_or_else(|| "Calibri".into());
        let title_font = font
            .map(|s| s.to_string())
            .unwrap_or_else(|| "Calibri Light".into());

        let mut theme = match name.to_lowercase().as_str() {
            "light" | "default" => Theme {
                name: "light",
                slide_w,
                slide_h,
                aspect_kind,
                portrait,
                title_size,
                body_size,
                code_size,
                hero_size,
                bg: "FFFFFF".into(),
                title_color: "0F172A".into(),
                body_color: "334155".into(),
                muted_color: "94A3B8".into(),
                accent: "0EA5E9".into(),
                accent_soft: "E0F2FE".into(),
                divider: "E2E8F0".into(),
                code_bg: "F1F5F9".into(),
                code_text: "1E293B".into(),
                code_accent: "0369A1".into(),
                section_bg: "0F172A".into(),
                section_text: "F8FAFC".into(),
                link: "0EA5E9".into(),
                on_accent: "FFFFFF".into(),
                title_center: false,
                title_font,
                body_font,
                mono_font: "Consolas".into(),
                pdf_font: None,
                pdf_mono_font: None,
                font_fallback: Vec::new(),
                syntax_keyword: "9333EA".into(),
                syntax_string: "16A34A".into(),
                syntax_number: "C2410C".into(),
                syntax_comment: "94A3B8".into(),
                syntax_function: "2563EB".into(),
                syntax_type: "0891B2".into(),
                syntax_attribute: "DC2626".into(),
            },
            "dark" | "night" => Theme {
                name: "dark",
                slide_w,
                slide_h,
                aspect_kind,
                portrait,
                title_size,
                body_size,
                code_size,
                hero_size,
                bg: "0B1220".into(),
                title_color: "F8FAFC".into(),
                body_color: "CBD5E1".into(),
                muted_color: "64748B".into(),
                accent: "38BDF8".into(),
                accent_soft: "0C4A6E".into(),
                divider: "1E293B".into(),
                code_bg: "111A2E".into(),
                code_text: "E2E8F0".into(),
                code_accent: "7DD3FC".into(),
                section_bg: "0EA5E9".into(),
                section_text: "FFFFFF".into(),
                link: "7DD3FC".into(),
                on_accent: "0B1220".into(),
                title_center: false,
                title_font,
                body_font,
                mono_font: "Consolas".into(),
                pdf_font: None,
                pdf_mono_font: None,
                font_fallback: Vec::new(),
                syntax_keyword: "C792EA".into(),
                syntax_string: "C3E88D".into(),
                syntax_number: "FFCB6B".into(),
                syntax_comment: "6B7B8E".into(),
                syntax_function: "82AAFF".into(),
                syntax_type: "FFC777".into(),
                syntax_attribute: "F78C6C".into(),
            },
            other => {
                // Named gallery themes are a palette layered on the light or
                // dark base, so they inherit all geometry/sizing automatically.
                if let Some((dark_base, static_name, palette)) = builtin_theme(other) {
                    let mut t =
                        Theme::resolve(if dark_base { "dark" } else { "light" }, aspect, font)?;
                    t.apply_override(&palette)?;
                    t.name = static_name;
                    // Re-derive code colours from the (possibly new) background.
                    t.apply_code_theme(CodeTheme::Match);
                    return Ok(t);
                }
                bail!("unknown theme: {} (try: {})", other, THEME_NAMES.join(", "))
            }
        };

        theme.apply_code_theme(CodeTheme::default());
        Ok(theme)
    }

    pub fn apply_code_theme(&mut self, code_theme: CodeTheme) {
        let target = match code_theme {
            CodeTheme::Dark => "dark",
            CodeTheme::Light => "light",
            // Pick by background luminance, not name, so the gallery themes
            // (and any custom palette) get code colours that match their bg.
            CodeTheme::Match => {
                if hex_luma(&self.bg).unwrap_or(1.0) < 0.45 {
                    "dark"
                } else {
                    "light"
                }
            }
        };
        match target {
            "dark" => {
                self.code_bg = "111A2E".into();
                self.code_text = "E2E8F0".into();
                self.code_accent = "7DD3FC".into();
                self.syntax_keyword = "C792EA".into();
                self.syntax_string = "C3E88D".into();
                self.syntax_number = "FFCB6B".into();
                self.syntax_comment = "6B7B8E".into();
                self.syntax_function = "82AAFF".into();
                self.syntax_type = "FFC777".into();
                self.syntax_attribute = "F78C6C".into();
            }
            _ => {
                self.code_bg = "F1F5F9".into();
                self.code_text = "1E293B".into();
                self.code_accent = "0369A1".into();
                self.syntax_keyword = "9333EA".into();
                self.syntax_string = "16A34A".into();
                self.syntax_number = "C2410C".into();
                self.syntax_comment = "94A3B8".into();
                self.syntax_function = "2563EB".into();
                self.syntax_type = "0891B2".into();
                self.syntax_attribute = "DC2626".into();
            }
        }
    }

    pub fn apply_override(&mut self, ov: &ThemeOverride) -> Result<()> {
        macro_rules! set_color {
            ($field:ident) => {
                if let Some(v) = &ov.$field {
                    self.$field = normalize_hex(v, stringify!($field))?;
                }
            };
        }
        set_color!(bg);
        set_color!(title_color);
        set_color!(body_color);
        set_color!(muted_color);
        set_color!(accent);
        set_color!(accent_soft);
        set_color!(divider);
        set_color!(code_bg);
        set_color!(code_text);
        set_color!(code_accent);
        set_color!(section_bg);
        set_color!(section_text);
        set_color!(link);
        set_color!(on_accent);

        if let Some(v) = &ov.title_align {
            self.title_center = v.eq_ignore_ascii_case("center");
        }

        if let Some(v) = &ov.title_font {
            self.title_font = v.clone();
        }
        if let Some(v) = &ov.body_font {
            self.body_font = v.clone();
        }
        if let Some(v) = &ov.mono_font {
            self.mono_font = v.clone();
        }
        if let Some(v) = &ov.pdf_font {
            self.pdf_font = Some(v.clone());
        }
        if let Some(v) = &ov.pdf_mono_font {
            self.pdf_mono_font = Some(v.clone());
        }
        if !ov.font_fallback.is_empty() {
            self.font_fallback = ov.font_fallback.clone();
        }

        if let Some(v) = ov.title_size {
            self.title_size = v;
        }
        if let Some(v) = ov.body_size {
            self.body_size = v;
        }
        if let Some(v) = ov.code_size {
            self.code_size = v;
        }
        if let Some(v) = ov.hero_size {
            self.hero_size = v;
        }

        if let Some(s) = &ov.syntax {
            if let Some(c) = &s.keyword {
                self.syntax_keyword = normalize_hex(c, "syntax.keyword")?;
            }
            if let Some(c) = &s.string {
                self.syntax_string = normalize_hex(c, "syntax.string")?;
            }
            if let Some(c) = &s.number {
                self.syntax_number = normalize_hex(c, "syntax.number")?;
            }
            if let Some(c) = &s.comment {
                self.syntax_comment = normalize_hex(c, "syntax.comment")?;
            }
            if let Some(c) = &s.function {
                self.syntax_function = normalize_hex(c, "syntax.function")?;
            }
            if let Some(c) = &s.type_color {
                self.syntax_type = normalize_hex(c, "syntax.type")?;
            }
            if let Some(c) = &s.attribute {
                self.syntax_attribute = normalize_hex(c, "syntax.attribute")?;
            }
        }
        Ok(())
    }
}

/// Parse a free-form aspect string into (width_emu, height_emu, kind, portrait).
///
/// Accepted forms:
///   `1920x1080`         — pixels at 96 DPI
///   `1920x1080px`       — explicit pixels
///   `300x200mm`         — millimetres
///   `30x20cm`           — centimetres
///   `13.33x7.5in`       — inches
///   `12700x7150emu`     — raw EMU (1 inch = 914,400 EMU)
///
/// Plain `WxH` with no unit suffix is interpreted as pixels at 96 DPI so the
/// common "I have a 1920x1080 slide" case works out of the box.
fn parse_custom_aspect(s: &str) -> Result<(u32, u32, &'static str, bool)> {
    let s = s.trim();
    if s.is_empty() {
        bail!("empty aspect string");
    }
    // Split on 'x' (preferred) or ':'. Last numeric segment may carry a unit.
    let sep_idx = s
        .char_indices()
        .filter(|(_, c)| *c == 'x' || *c == 'X' || *c == ':')
        .map(|(i, _)| i)
        .next()
        .ok_or_else(|| anyhow::anyhow!(
            "unknown aspect: {} (try 16:9, 4:3, 9:16, a4, letter, or `WxH[unit]` like 1920x1080px or 300x200mm)",
            s
        ))?;
    let left = &s[..sep_idx];
    let rest = &s[sep_idx + 1..];

    // Trailing unit suffix: scan from the right while we see ASCII letters.
    let unit_start = rest.len()
        - rest
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_alphabetic())
            .count();
    let (right_num, unit) = rest.split_at(unit_start);
    let unit = unit.to_ascii_lowercase();

    let w_val: f64 = left
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid width in aspect {:?}", s))?;
    let h_val: f64 = right_num
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid height in aspect {:?}", s))?;
    if w_val <= 0.0 || h_val <= 0.0 {
        bail!("aspect dimensions must be positive: {:?}", s);
    }

    // EMU per unit. EMU = English Metric Units; 1 in = 914,400 EMU.
    let emu_per_unit: f64 = match unit.as_str() {
        "" | "px" | "pixel" | "pixels" => 914_400.0 / 96.0,
        "in" | "inch" | "inches" => 914_400.0,
        "mm" => 36_000.0,
        "cm" => 360_000.0,
        "emu" => 1.0,
        "pt" | "points" => 12_700.0,
        other => bail!(
            "unknown unit {:?} in aspect (try px, mm, cm, in, pt, emu)",
            other
        ),
    };

    let w_emu = (w_val * emu_per_unit).round() as u32;
    let h_emu = (h_val * emu_per_unit).round() as u32;
    if w_emu < 100_000 || h_emu < 100_000 {
        bail!(
            "aspect {:?} is too small (< 0.1 in × 0.1 in once converted)",
            s
        );
    }
    let portrait = h_emu > w_emu;
    Ok((w_emu, h_emu, "custom", portrait))
}

pub fn load_override(path: &Path) -> Result<ThemeOverride> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read theme override {}", path.display()))?;
    let ov: ThemeOverride = serde_yaml::from_str(&text)
        .with_context(|| format!("parse theme override {}", path.display()))?;
    Ok(ov)
}
