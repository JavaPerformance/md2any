//! Browser bindings for the md2any conversion engine.
//!
//! All work runs in the tab (or a Web Worker). Markdown and optional image
//! assets are passed in; Office/PDF/HTML bytes come out. Nothing is uploaded.

use md2any::convert::{self, ConvertOptions, OutputFormat};
use md2any::math::MathMode;
use serde::Serialize;
use std::collections::HashMap;
use wasm_bindgen::prelude::*;

/// Install a panic hook that logs to the browser console (optional feature).
#[wasm_bindgen(start)]
pub fn start() {
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();
}

/// Engine metadata for the UI.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Built-in theme names as a JS array of strings.
#[wasm_bindgen(js_name = themeNames)]
pub fn theme_names() -> JsValue {
    serde_wasm_bindgen::to_value(convert::theme_names()).unwrap_or(JsValue::NULL)
}

/// Built-in layout names as a JS array of strings.
#[wasm_bindgen(js_name = layoutNames)]
pub fn layout_names() -> JsValue {
    serde_wasm_bindgen::to_value(convert::layout_names()).unwrap_or(JsValue::NULL)
}

#[derive(Serialize)]
struct OutlineSlideJson {
    index: usize,
    title: String,
    kind: String,
    #[serde(rename = "sourceLine")]
    source_line: u32,
    #[serde(rename = "hasNotes")]
    has_notes: bool,
    notes: Option<String>,
}

#[derive(Serialize)]
struct ConvertResultJson {
    /// Base64-encoded file bytes (binary-safe across the JS bridge).
    base64: String,
    filename: String,
    #[serde(rename = "contentType")]
    content_type: String,
    #[serde(rename = "slideCount")]
    slide_count: usize,
    title: String,
}

/// Parse markdown and return a slide outline for the rail UI.
///
/// `assets_json` is optional: a JSON object mapping image path → base64 bytes.
#[wasm_bindgen]
pub fn outline(
    markdown: &str,
    theme: Option<String>,
    aspect: Option<String>,
    layout: Option<String>,
    assets_json: Option<String>,
) -> Result<JsValue, JsValue> {
    let assets = parse_assets(assets_json.as_deref())?;
    let opts = ConvertOptions {
        format: OutputFormat::Html,
        theme,
        aspect,
        layout,
        ..Default::default()
    };
    let slides = convert::outline(markdown, &opts, &assets).map_err(err)?;
    let json: Vec<OutlineSlideJson> = slides
        .into_iter()
        .map(|s| OutlineSlideJson {
            index: s.index,
            title: s.title,
            kind: s.kind,
            source_line: s.source_line,
            has_notes: s.has_notes,
            notes: s.notes,
        })
        .collect();
    serde_wasm_bindgen::to_value(&json).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Live HTML preview of the deck (full document — legacy / fallback path).
#[wasm_bindgen(js_name = previewHtml)]
pub fn preview_html(
    markdown: &str,
    theme: Option<String>,
    aspect: Option<String>,
    layout: Option<String>,
    assets_json: Option<String>,
) -> Result<String, JsValue> {
    let assets = parse_assets(assets_json.as_deref())?;
    let opts = ConvertOptions {
        format: OutputFormat::Html,
        theme,
        aspect,
        layout,
        editor_preview: true,
        math_mode: MathMode::Unicode,
        ..Default::default()
    };
    let result = convert::convert(markdown, &opts, &assets).map_err(err)?;
    String::from_utf8(result.bytes).map_err(|e| JsValue::from_str(&e.to_string()))
}

#[derive(Serialize)]
struct PreviewSlideJson {
    index: usize,
    title: String,
    kind: String,
    #[serde(rename = "sourceLine")]
    source_line: u32,
    html: Option<String>,
    #[serde(rename = "contentKey")]
    content_key: String,
    #[serde(rename = "hasNotes")]
    has_notes: bool,
    notes: Option<String>,
}

#[derive(Serialize)]
struct PreviewWindowJson {
    title: String,
    #[serde(rename = "slideCount")]
    slide_count: usize,
    #[serde(rename = "bodyClass")]
    body_class: String,
    css: String,
    #[serde(rename = "structureKey")]
    structure_key: String,
    slides: Vec<PreviewSlideJson>,
    #[serde(rename = "htmlFrom")]
    html_from: usize,
    #[serde(rename = "htmlTo")]
    html_to: usize,
}

/// Virtualised studio preview: outline for **all** slides, HTML only for
/// `html_from..html_to` (0-based, end exclusive).
#[wasm_bindgen(js_name = previewWindow)]
pub fn preview_window(
    markdown: &str,
    theme: Option<String>,
    aspect: Option<String>,
    layout: Option<String>,
    assets_json: Option<String>,
    html_from: u32,
    html_to: u32,
) -> Result<JsValue, JsValue> {
    let assets = parse_assets(assets_json.as_deref())?;
    let opts = ConvertOptions {
        format: OutputFormat::Html,
        theme,
        aspect,
        layout,
        editor_preview: true,
        math_mode: MathMode::Unicode,
        ..Default::default()
    };
    let win = convert::preview_window(
        markdown,
        &opts,
        &assets,
        html_from as usize,
        html_to as usize,
    )
    .map_err(err)?;
    let json = PreviewWindowJson {
        title: win.title,
        slide_count: win.slide_count,
        body_class: win.body_class,
        css: win.css,
        structure_key: win.structure_key,
        slides: win
            .slides
            .into_iter()
            .map(|s| PreviewSlideJson {
                index: s.index,
                title: s.title,
                kind: s.kind,
                source_line: s.source_line,
                html: s.html,
                content_key: s.content_key,
                has_notes: s.has_notes,
                notes: s.notes,
            })
            .collect(),
        html_from: win.html_from,
        html_to: win.html_to,
    };
    serde_wasm_bindgen::to_value(&json).map_err(|e| JsValue::from_str(&e.to_string()))
}

#[derive(Serialize)]
struct SlideImageJson {
    index: usize,
    title: String,
    format: String,
    /// Base64-encoded SVG or PNG bytes.
    base64: String,
}

/// Export-geometry slide images (SVG or PNG) for a window of slides.
///
/// Same IR path as CLI `--format svg|png`. Studio uses this for the export
/// ghost (true layout fidelity) and filmstrip thumbnails.
#[wasm_bindgen(js_name = slideImages)]
pub fn slide_images(
    markdown: &str,
    theme: Option<String>,
    aspect: Option<String>,
    layout: Option<String>,
    assets_json: Option<String>,
    from: u32,
    to: u32,
    format: Option<String>,
) -> Result<JsValue, JsValue> {
    let assets = parse_assets(assets_json.as_deref())?;
    let opts = ConvertOptions {
        format: OutputFormat::Html,
        theme,
        aspect,
        layout,
        editor_preview: true,
        math_mode: MathMode::Unicode,
        ..Default::default()
    };
    let fmt = format.as_deref().unwrap_or("svg");
    let imgs = convert::slide_images(
        markdown,
        &opts,
        &assets,
        from as usize,
        to as usize,
        fmt,
    )
    .map_err(err)?;
    let json: Vec<SlideImageJson> = imgs
        .into_iter()
        .map(|s| SlideImageJson {
            index: s.index,
            title: s.title,
            format: s.format,
            base64: base64_encode(&s.bytes),
        })
        .collect();
    serde_wasm_bindgen::to_value(&json).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Run deck lints (`--check` equivalent) and return `{slide, kind, detail}[]`.
#[wasm_bindgen]
pub fn lint(
    markdown: &str,
    theme: Option<String>,
    aspect: Option<String>,
    layout: Option<String>,
    assets_json: Option<String>,
) -> Result<JsValue, JsValue> {
    let assets = parse_assets(assets_json.as_deref())?;
    let opts = ConvertOptions {
        format: OutputFormat::Html,
        theme,
        aspect,
        layout,
        editor_preview: true,
        math_mode: MathMode::Unicode,
        ..Default::default()
    };
    let hits = convert::lint(markdown, &opts, &assets).map_err(err)?;
    #[derive(Serialize)]
    struct Hit {
        slide: usize,
        kind: String,
        detail: String,
    }
    let json: Vec<Hit> = hits
        .into_iter()
        .map(|h| Hit {
            slide: h.slide,
            kind: h.kind,
            detail: h.detail,
        })
        .collect();
    serde_wasm_bindgen::to_value(&json).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Extract a brand YAML overlay from a `.potx` / `.pptx` (base64 package bytes).
///
/// Same mapping as CLI `md2any theme extract`. The YAML can be applied as a
/// front-matter `style:` block or saved as `brand.yaml`.
#[wasm_bindgen(js_name = extractBrand)]
pub fn extract_brand(base64_pptx: &str, source_name: Option<String>) -> Result<String, JsValue> {
    let bytes = base64_decode(base64_pptx)
        .map_err(|e| JsValue::from_str(&format!("potx base64: {e}")))?;
    let source = source_name.unwrap_or_else(|| "template.potx".into());
    md2any::brandkit::extract_overlay_bytes(&bytes, &source).map_err(err)
}

/// Convert markdown to a downloadable file.
///
/// `format` is one of: pptx, odp, pdf, docx, odt, html.
/// Returns `{ base64, filename, contentType, slideCount, title }`.
#[wasm_bindgen]
pub fn convert(
    markdown: &str,
    format: &str,
    theme: Option<String>,
    aspect: Option<String>,
    layout: Option<String>,
    assets_json: Option<String>,
) -> Result<JsValue, JsValue> {
    let assets = parse_assets(assets_json.as_deref())?;
    let fmt = OutputFormat::from_name(format).map_err(err)?;
    let opts = ConvertOptions {
        format: fmt,
        theme,
        aspect,
        layout,
        editor_preview: false,
        math_mode: MathMode::Unicode,
        ..Default::default()
    };
    let result = convert::convert(markdown, &opts, &assets).map_err(err)?;
    let json = ConvertResultJson {
        base64: base64_encode(&result.bytes),
        filename: result.filename,
        content_type: result.content_type,
        slide_count: result.slide_count,
        title: result.title,
    };
    serde_wasm_bindgen::to_value(&json).map_err(|e| JsValue::from_str(&e.to_string()))
}

fn parse_assets(assets_json: Option<&str>) -> Result<HashMap<String, Vec<u8>>, JsValue> {
    let Some(raw) = assets_json.filter(|s| !s.trim().is_empty() && s.trim() != "{}") else {
        return Ok(HashMap::new());
    };
    let map: HashMap<String, String> =
        serde_json::from_str(raw).map_err(|e| JsValue::from_str(&format!("assets JSON: {e}")))?;
    let mut out = HashMap::with_capacity(map.len());
    for (k, b64) in map {
        let bytes = base64_decode(&b64)
            .map_err(|e| JsValue::from_str(&format!("asset '{k}' base64: {e}")))?;
        out.insert(k, bytes);
    }
    Ok(out)
}

fn err(e: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&e.to_string())
}

// Minimal base64 (avoid pulling in another crate just for the bridge).
fn base64_encode(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(T[((n >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(T[(n & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    fn val(c: u8) -> Result<u8, String> {
        match c {
            b'A'..=b'Z' => Ok(c - b'A'),
            b'a'..=b'z' => Ok(c - b'a' + 26),
            b'0'..=b'9' => Ok(c - b'0' + 52),
            b'+' => Ok(62),
            b'/' => Ok(63),
            _ => Err(format!("invalid base64 byte {}", c)),
        }
    }
    let bytes: Vec<u8> = input
        .bytes()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();
    if bytes.len() % 4 != 0 {
        return Err("invalid base64 length".into());
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let (a, b, c, d) = (chunk[0], chunk[1], chunk[2], chunk[3]);
        let n = (val(a)? as u32) << 18
            | (val(b)? as u32) << 12
            | (if c == b'=' { 0 } else { val(c)? as u32 }) << 6
            | if d == b'=' { 0 } else { val(d)? as u32 };
        out.push(((n >> 16) & 0xff) as u8);
        if c != b'=' {
            out.push(((n >> 8) & 0xff) as u8);
        }
        if d != b'=' {
            out.push((n & 0xff) as u8);
        }
    }
    Ok(out)
}
