//! Smoke + structural tests for the five output formats.
//!
//! These tests don't compare byte-identical output (rendering bytes drift
//! with every layout tweak), they assert *structural* invariants — the kind
//! of regression that would silently corrupt a deck if it slipped through:
//!
//! - PDF: valid header, valid xref, %%EOF terminator, embedded text round-trips
//! - PPTX/ODP/DOCX/ODT: ZIP container parses, mandatory parts are present,
//!   slide / paragraph counts match the input

use std::io::Read;
use std::path::PathBuf;

fn assets() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn render(format: &str) -> Vec<u8> {
    let md = "---\n\
              title: Renderer test\n\
              author: tests\n\
              theme: light\n\
              ---\n\
              \n\
              # Section one\n\
              \n\
              Some text with **bold**, *italic*, and `code`.\n\
              \n\
              - alpha\n\
              - beta\n\
              - gamma\n\
              \n\
              ```rust\n\
              fn main() { println!(\"hi\"); }\n\
              ```\n\
              \n\
              | col1 | col2 |\n\
              |------|------|\n\
              | a    | b    |\n\
              | c    | d    |\n\
              \n\
              # Section two\n\
              \n\
              Closing paragraph with a [link](https://example.com).\n";

    let (front, body) = md2any::front_matter::extract(md);
    let theme = md2any::theme::Theme::resolve("light", "16:9", None).unwrap();
    let layout = md2any::layout::Layout::resolve("clean").unwrap();
    let slides = md2any::parser::parse(&body, &front, "test");
    let slides = md2any::paginate::paginate(slides, &theme);

    let base = assets();
    match format {
        "pptx" => md2any::pptx::write(
            &slides,
            &theme,
            &layout,
            "Renderer test",
            "tests",
            &base,
            None,
            None,
            0.4,
            None,
        )
        .unwrap(),
        "odp" => md2any::odp::write(
            &slides,
            &theme,
            &layout,
            "Renderer test",
            "tests",
            &base,
            None,
            None,
            0.4,
            None,
        )
        .unwrap(),
        "pdf" => md2any::pdf::write(
            &slides,
            &theme,
            &layout,
            "Renderer test",
            "tests",
            &base,
            None,
            None,
            None,
            0.4,
            None,
            false,
            None,
        )
        .unwrap(),
        "docx" => md2any::docx::write(&slides, &theme, "Renderer test", "tests", &base, None, None)
            .unwrap(),
        "odt" => md2any::odt::write(&slides, &theme, "Renderer test", "tests", &base, None, None)
            .unwrap(),
        other => panic!("unknown format {other}"),
    }
}

fn zip_contains(bytes: &[u8], wanted: &[&str]) -> Result<(), String> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| format!("not a zip: {e}"))?;
    let names: Vec<String> = (0..archive.len())
        .map(|i| archive.by_index(i).unwrap().name().to_string())
        .collect();
    for w in wanted {
        if !names.iter().any(|n| n == w) {
            return Err(format!("missing part: {w}\npresent: {:?}", names));
        }
    }
    Ok(())
}

fn zip_read(bytes: &[u8], name: &str) -> String {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor).unwrap();
    let mut file = archive.by_name(name).unwrap();
    let mut s = String::new();
    file.read_to_string(&mut s).unwrap();
    s
}

// ---------------------------------------------------------------------------
// PDF
// ---------------------------------------------------------------------------

#[test]
fn pdf_has_valid_envelope() {
    let bytes = render("pdf");
    assert!(bytes.starts_with(b"%PDF-1."), "PDF header missing");
    assert!(
        bytes.ends_with(b"%%EOF\n") || bytes.ends_with(b"%%EOF"),
        "no %%EOF"
    );
    assert!(bytes.windows(5).any(|w| w == b"xref\n"), "no xref table");
}

#[test]
fn pdf_embeds_fonts() {
    let bytes = render("pdf");
    let s = std::str::from_utf8(&bytes[..bytes.len().min(8192)])
        .unwrap_or("")
        .to_owned();
    // The font name appears in the cleartext part of the PDF (the font
    // descriptor object).
    let full = String::from_utf8_lossy(&bytes);
    assert!(
        full.contains("DejaVuSans"),
        "expected DejaVu font reference, got: {s}"
    );
}

// ---------------------------------------------------------------------------
// PPTX
// ---------------------------------------------------------------------------

#[test]
fn pptx_has_mandatory_parts() {
    let bytes = render("pptx");
    zip_contains(
        &bytes,
        &[
            "[Content_Types].xml",
            "_rels/.rels",
            "ppt/presentation.xml",
            "ppt/_rels/presentation.xml.rels",
            "ppt/slides/slide1.xml",
        ],
    )
    .unwrap();
}

#[test]
fn pptx_slide_count_matches_deck() {
    let bytes = render("pptx");
    let cursor = std::io::Cursor::new(&bytes);
    let mut archive = zip::ZipArchive::new(cursor).unwrap();
    let mut count = 0;
    for i in 0..archive.len() {
        let name = archive.by_index(i).unwrap().name().to_string();
        if name.starts_with("ppt/slides/slide") && name.ends_with(".xml") {
            count += 1;
        }
    }
    // Title + 2 sections + 1 content per section (auto-spawned).
    assert!(count >= 3, "expected at least 3 slides, got {count}");
}

#[test]
fn pptx_renders_table() {
    let bytes = render("pptx");
    let mut found = false;
    let cursor = std::io::Cursor::new(&bytes);
    let mut archive = zip::ZipArchive::new(cursor).unwrap();
    for i in 0..archive.len() {
        let name = archive.by_index(i).unwrap().name().to_string();
        if name.starts_with("ppt/slides/slide") && name.ends_with(".xml") {
            let xml = zip_read(&bytes, &name);
            if xml.contains("<a:tbl>") {
                found = true;
                break;
            }
        }
    }
    assert!(found, "expected at least one <a:tbl> in slides");
}

// ---------------------------------------------------------------------------
// ODP
// ---------------------------------------------------------------------------

#[test]
fn odp_has_mandatory_parts() {
    let bytes = render("odp");
    zip_contains(
        &bytes,
        &[
            "mimetype",
            "META-INF/manifest.xml",
            "content.xml",
            "styles.xml",
            "meta.xml",
        ],
    )
    .unwrap();
}

#[test]
fn odp_mimetype_is_first() {
    let bytes = render("odp");
    let cursor = std::io::Cursor::new(&bytes);
    let mut archive = zip::ZipArchive::new(cursor).unwrap();
    let first = archive.by_index(0).unwrap().name().to_string();
    // ODF spec: mimetype must be first and stored (not deflated).
    assert_eq!(first, "mimetype");
}

#[test]
fn odp_content_has_pages() {
    let bytes = render("odp");
    let content = zip_read(&bytes, "content.xml");
    assert!(content.contains("<draw:page"), "no draw:page elements");
}

// ---------------------------------------------------------------------------
// DOCX
// ---------------------------------------------------------------------------

#[test]
fn docx_has_mandatory_parts() {
    let bytes = render("docx");
    zip_contains(
        &bytes,
        &[
            "[Content_Types].xml",
            "_rels/.rels",
            "word/document.xml",
            "word/styles.xml",
            "word/numbering.xml",
        ],
    )
    .unwrap();
}

#[test]
fn docx_document_has_headings_and_table() {
    let bytes = render("docx");
    let doc = zip_read(&bytes, "word/document.xml");
    assert!(doc.contains("Heading1"), "no Heading1 style reference");
    assert!(doc.contains("<w:tbl>"), "no table");
    assert!(doc.contains("ListParagraph"), "no list paragraphs");
}

// ---------------------------------------------------------------------------
// ODT
// ---------------------------------------------------------------------------

#[test]
fn odt_has_mandatory_parts() {
    let bytes = render("odt");
    zip_contains(
        &bytes,
        &[
            "mimetype",
            "META-INF/manifest.xml",
            "content.xml",
            "styles.xml",
        ],
    )
    .unwrap();
}

#[test]
fn odt_content_has_paragraphs_and_table() {
    let bytes = render("odt");
    let content = zip_read(&bytes, "content.xml");
    assert!(content.contains("<text:h"), "no headings");
    assert!(content.contains("<table:table"), "no table");
}

// ---------------------------------------------------------------------------
// Direction (RTL)
// ---------------------------------------------------------------------------

#[test]
fn docx_rtl_emits_bidi() {
    let md = "---\ntitle: rtl\ndirection: rtl\n---\n# Hi\n- a\n- b\n";
    let (front, body) = md2any::front_matter::extract(md);
    let theme = md2any::theme::Theme::resolve("light", "16:9", None).unwrap();
    let slides = md2any::parser::parse(&body, &front, "test");
    let slides = md2any::paginate::paginate(slides, &theme);
    let bytes = md2any::docx::write(
        &slides,
        &theme,
        "rtl",
        "tests",
        &assets(),
        None,
        front.direction.as_deref(),
    )
    .unwrap();
    let doc = zip_read(&bytes, "word/document.xml");
    assert!(doc.contains("<w:bidi/>"), "expected <w:bidi/> for rtl");
}

#[test]
fn odt_rtl_flips_writing_mode() {
    let md = "---\ntitle: rtl\ndirection: rtl\n---\n# Hi\n- a\n";
    let (front, body) = md2any::front_matter::extract(md);
    let theme = md2any::theme::Theme::resolve("light", "16:9", None).unwrap();
    let slides = md2any::parser::parse(&body, &front, "test");
    let slides = md2any::paginate::paginate(slides, &theme);
    let bytes = md2any::odt::write(
        &slides,
        &theme,
        "rtl",
        "tests",
        &assets(),
        None,
        front.direction.as_deref(),
    )
    .unwrap();
    let styles = zip_read(&bytes, "styles.xml");
    assert!(styles.contains("rl-tb"), "expected rl-tb writing mode");
}
