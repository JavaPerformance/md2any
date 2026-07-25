//! Brand-kit helpers: derive an md2any theme overlay from existing assets.
//!
//! Corporate decks usually start from a PowerPoint template (`.potx` /
//! `.pptx`). md2any is a *generator*, not a template host — it never pours
//! content into someone else's slide masters. But the part of a template
//! that actually carries the brand — the theme's **colour scheme** and
//! **font scheme** — lives in a single, well-specified XML part
//! (`ppt/theme/theme1.xml`, ECMA-376 §20.1.6). This module reads that part
//! and emits a starter `brand.yaml` overlay (a [`crate::theme::ThemeOverride`])
//! that `--theme-file` consumes, so a team can go from "here's our `.potx`"
//! to "here's our md2any brand" in seconds.
//!
//! The mapping is a sensible starting point, not a pixel-perfect import:
//! PowerPoint's colour scheme has more slots than a deck needs, and the
//! `dk1/lt1` ↔ background/text relationship is mediated by each master's
//! colour map. The emitted YAML is commented so it's easy to tweak.

use anyhow::{Context, Result};
use std::io::{Cursor, Read};
use std::path::Path;

/// Read a `.potx`/`.pptx` from disk and produce a commented `brand.yaml` overlay.
pub fn extract_overlay(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("open {}", path.display()))?;
    extract_overlay_bytes(&bytes, &path.display().to_string())
}

/// In-memory form of [`extract_overlay`] — used by the WASM studio and tests.
///
/// `source` is only used in the generated YAML header comment.
pub fn extract_overlay_bytes(bytes: &[u8], source: &str) -> Result<String> {
    let xml = read_theme_xml_bytes(bytes)
        .with_context(|| format!("read theme from {source}"))?;
    Ok(build_overlay(&xml, source))
}

/// Open a PowerPoint package (bytes) and return the first theme part's XML
/// (prefers `theme1.xml`, the part the primary slide master references).
fn read_theme_xml_bytes(bytes: &[u8]) -> Result<String> {
    let cursor = Cursor::new(bytes);
    let mut zip = zip::ZipArchive::new(cursor).with_context(|| {
        "not a valid PowerPoint package (.potx/.pptx are ZIP archives)".to_string()
    })?;

    let name = pick_theme_name(&mut zip)?;
    let mut f = zip
        .by_name(&name)
        .with_context(|| format!("read {name}"))?;
    let mut xml = String::new();
    f.read_to_string(&mut xml)
        .with_context(|| format!("decode {name}"))?;
    Ok(xml)
}

fn pick_theme_name<R: Read + std::io::Seek>(zip: &mut zip::ZipArchive<R>) -> Result<String> {
    if zip.by_name("ppt/theme/theme1.xml").is_ok() {
        return Ok("ppt/theme/theme1.xml".to_string());
    }
    for i in 0..zip.len() {
        let n = zip.by_index(i)?.name().to_string();
        if n.starts_with("ppt/theme/") && n.ends_with(".xml") {
            return Ok(n);
        }
    }
    anyhow::bail!(
        "no ppt/theme/themeN.xml found — is this a PowerPoint .pptx/.potx? \
         (Keynote/Google Slides exports won't have one)"
    )
}

/// Build the commented overlay from a theme XML string. Split out from I/O so
/// it's unit-testable against synthetic fixtures.
fn build_overlay(xml: &str, source: &str) -> String {
    // Colour scheme slots (ECMA-376 §20.1.6.2). dk1/lt1 are usually system
    // colours (window / windowText); dk2/lt2/accentN/hlink are sRGB.
    let dk1 = color_slot(xml, "dk1").unwrap_or_else(|| "000000".to_string());
    let lt1 = color_slot(xml, "lt1").unwrap_or_else(|| "FFFFFF".to_string());
    let dk2 = color_slot(xml, "dk2");
    let lt2 = color_slot(xml, "lt2");
    let accent1 = color_slot(xml, "accent1");
    let accent2 = color_slot(xml, "accent2");
    let hlink = color_slot(xml, "hlink");

    // Font scheme: major (headings) and minor (body) Latin typefaces.
    let major = font_slot(xml, "majorFont");
    let minor = font_slot(xml, "minorFont");

    // A light brand has a light background (lt1); pair the overlay with the
    // matching base --theme so non-overridden tokens (code, etc.) fit.
    let base = if luma(&lt1).unwrap_or(1.0) >= 0.5 {
        "light"
    } else {
        "dark"
    };

    let title = dk2.clone().unwrap_or_else(|| dk1.clone());

    let mut out = String::new();
    out.push_str(&format!(
        "# md2any brand overlay — extracted from {source}\n\
         #\n\
         # Use it with any deck, in every output format:\n\
         #   md2any deck.md --theme {base} --theme-file brand.yaml -o deck.pptx\n\
         #\n\
         # This is a STARTING POINT mapped from the template's theme part\n\
         # (colour scheme + font scheme). Tweak the values to taste; only the\n\
         # keys present here are applied, everything else inherits --theme {base}.\n\
         # Add a footer logo with --logo your-logo.png.\n\n"
    ));

    out.push_str("# --- Colours (from the template's clrScheme) ---\n");
    push_color(&mut out, "bg", &lt1, "lt1 — slide background");
    push_color(&mut out, "body_color", &dk1, "dk1 — body text");
    push_color(&mut out, "title_color", &title, "dk2 — slide titles");
    if let Some(c) = &accent1 {
        push_color(&mut out, "accent", c, "accent1 — primary brand colour");
    }
    if let Some(c) = &accent2 {
        push_color(&mut out, "accent_soft", c, "accent2 — secondary / card fill");
    }
    if let Some(c) = &lt2 {
        push_color(&mut out, "divider", c, "lt2 — rules / dividers");
    }
    if let Some(c) = &hlink {
        push_color(&mut out, "link", c, "hlink — hyperlinks");
    }
    if let Some(c) = &accent1 {
        push_color(&mut out, "section_bg", c, "section-divider background");
        push_color(&mut out, "on_accent", &lt1, "text drawn on the accent");
    }

    out.push_str("\n# --- Fonts (from the template's fontScheme) ---\n");
    out.push_str("# Named fonts apply to PPTX/ODP/DOCX/ODT and resolve on the\n");
    out.push_str("# opener's machine. For self-contained PDF output, also point\n");
    out.push_str("# pdf_font / pdf_mono_font at .ttf/.otf files you can ship.\n");
    match &major {
        Some(f) => out.push_str(&format!("title_font: \"{f}\"   # majorFont\n")),
        None => out.push_str("# title_font: \"Your Heading Font\"\n"),
    }
    match &minor {
        Some(f) => out.push_str(&format!("body_font: \"{f}\"    # minorFont\n")),
        None => out.push_str("# body_font: \"Your Body Font\"\n"),
    }
    out.push_str("# mono_font: \"Consolas\"        # no theme equivalent — set if your brand has one\n");
    out.push_str("# pdf_font: \"fonts/Brand.ttf\"\n");
    out.push_str("# pdf_mono_font: \"fonts/BrandMono.ttf\"\n");

    out
}

fn push_color(out: &mut String, key: &str, hex: &str, note: &str) {
    // Quote the value: a bare #hex is a YAML comment.
    out.push_str(&format!("{key}: \"#{hex}\"   # {note}\n"));
}

/// Extract the resolved hex (uppercase, no `#`) for a named colour-scheme slot
/// such as `accent1` or `dk1`. Handles both `<a:srgbClr val="..."/>` and the
/// system colours `<a:sysClr val="window..." lastClr="..."/>`.
fn color_slot(xml: &str, name: &str) -> Option<String> {
    let block = slot_block(xml, name)?;
    if let Some(v) = attr_after(block, "srgbClr", "val") {
        return normalize_hex(&v);
    }
    if let Some(last) = attr_after(block, "sysClr", "lastClr") {
        return normalize_hex(&last);
    }
    // sysClr with no resolved lastClr: fall back to the well-known values.
    match attr_after(block, "sysClr", "val")?.as_str() {
        "window" => Some("FFFFFF".to_string()),
        "windowText" => Some("000000".to_string()),
        _ => None,
    }
}

/// First Latin `typeface` inside a `majorFont` / `minorFont` slot.
fn font_slot(xml: &str, name: &str) -> Option<String> {
    let block = slot_block(xml, name)?;
    let f = attr_after(block, "latin", "typeface")?;
    let f = f.trim();
    if f.is_empty() {
        None
    } else {
        Some(f.to_string())
    }
}

/// Return the inner XML of the element whose local name is `name` (namespace
/// prefix agnostic). Relies on the regular, non-nested shape of theme slots:
/// the opening `name>` and its closing `name>` bracket the content.
fn slot_block<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("{name}>");
    let open = xml.find(&key)?;
    let body_start = open + key.len();
    let end = xml[body_start..]
        .find(&key)
        .map(|r| body_start + r)
        .unwrap_or(xml.len());
    Some(&xml[body_start..end])
}

/// Read `attr="value"` that appears after the first occurrence of `tag`.
fn attr_after(s: &str, tag: &str, attr: &str) -> Option<String> {
    let t = s.find(tag)?;
    let rest = &s[t..];
    let key = format!("{attr}=\"");
    let a = rest.find(&key)? + key.len();
    let end = rest[a..].find('"')?;
    Some(rest[a..a + end].to_string())
}

/// Validate/uppercase a hex colour to 6 digits (`#` optional, `RGB` expanded).
fn normalize_hex(s: &str) -> Option<String> {
    let t = s.trim().trim_start_matches('#');
    if !t.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match t.len() {
        6 => Some(t.to_ascii_uppercase()),
        3 => Some(t.chars().flat_map(|c| [c, c]).collect::<String>().to_ascii_uppercase()),
        _ => None,
    }
}

fn luma(hex: &str) -> Option<f32> {
    let t = hex.trim().trim_start_matches('#');
    if t.len() != 6 || !t.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&t[0..2], 16).ok()? as f32 / 255.0;
    let g = u8::from_str_radix(&t[2..4], 16).ok()? as f32 / 255.0;
    let b = u8::from_str_radix(&t[4..6], 16).ok()? as f32 / 255.0;
    Some(0.2126 * r + 0.7152 * g + 0.0722 * b)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"<a:theme><a:themeElements>
      <a:clrScheme name="Acme">
        <a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1>
        <a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>
        <a:dk2><a:srgbClr val="44546A"/></a:dk2>
        <a:lt2><a:srgbClr val="E7E6E6"/></a:lt2>
        <a:accent1><a:srgbClr val="4472C4"/></a:accent1>
        <a:accent2><a:srgbClr val="ED7D31"/></a:accent2>
        <a:hlink><a:srgbClr val="0563C1"/></a:hlink>
        <a:folHlink><a:srgbClr val="954F72"/></a:folHlink>
      </a:clrScheme>
      <a:fontScheme name="Acme">
        <a:majorFont><a:latin typeface="Georgia"/></a:majorFont>
        <a:minorFont><a:latin typeface="Verdana"/></a:minorFont>
      </a:fontScheme>
    </a:themeElements></a:theme>"#;

    #[test]
    fn extracts_scheme_colours() {
        assert_eq!(color_slot(FIXTURE, "accent1").as_deref(), Some("4472C4"));
        assert_eq!(color_slot(FIXTURE, "accent2").as_deref(), Some("ED7D31"));
        assert_eq!(color_slot(FIXTURE, "dk2").as_deref(), Some("44546A"));
        assert_eq!(color_slot(FIXTURE, "hlink").as_deref(), Some("0563C1"));
    }

    #[test]
    fn system_colours_resolve_via_lastclr() {
        assert_eq!(color_slot(FIXTURE, "dk1").as_deref(), Some("000000"));
        assert_eq!(color_slot(FIXTURE, "lt1").as_deref(), Some("FFFFFF"));
    }

    #[test]
    fn hlink_not_confused_with_folhlink() {
        // folHlink has a capital H, so a lowercase "hlink>" search must hit hlink.
        assert_eq!(color_slot(FIXTURE, "hlink").as_deref(), Some("0563C1"));
    }

    #[test]
    fn extracts_fonts() {
        assert_eq!(font_slot(FIXTURE, "majorFont").as_deref(), Some("Georgia"));
        assert_eq!(font_slot(FIXTURE, "minorFont").as_deref(), Some("Verdana"));
    }

    #[test]
    fn overlay_is_valid_yaml_and_round_trips() {
        let yaml = build_overlay(FIXTURE, "acme.potx");
        // The emitted overlay must parse back into a ThemeOverride.
        let ov = crate::theme::load_override_str(&yaml).expect("overlay parses");
        // Values keep the leading '#' here; apply_override normalises later.
        assert_eq!(ov.accent.as_deref(), Some("#4472C4"));
        assert_eq!(ov.bg.as_deref(), Some("#FFFFFF"));
        assert_eq!(ov.title_font.as_deref(), Some("Georgia"));
        assert_eq!(ov.body_font.as_deref(), Some("Verdana"));
    }

    #[test]
    fn extract_overlay_bytes_reads_theme_part() {
        // Minimal OOXML package with a theme part — enough for the studio WASM path.
        let mut buf = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::FileOptions::default();
            zip.start_file("ppt/theme/theme1.xml", opts).unwrap();
            use std::io::Write;
            zip.write_all(FIXTURE.as_bytes()).unwrap();
            zip.finish().unwrap();
        }
        let bytes = buf.into_inner();
        let yaml = extract_overlay_bytes(&bytes, "fixture.potx").expect("extract");
        let ov = crate::theme::load_override_str(&yaml).expect("parse");
        assert_eq!(ov.accent.as_deref(), Some("#4472C4"));
        assert_eq!(ov.title_font.as_deref(), Some("Georgia"));
    }
}
