//! Smoke + structural tests for the eight output formats.
//!
//! These tests don't compare byte-identical output (rendering bytes drift
//! with every layout tweak), they assert *structural* invariants — the kind
//! of regression that would silently corrupt a deck if it slipped through:
//!
//! - PDF: valid header, valid xref, %%EOF terminator, embedded text round-trips
//! - PPTX/ODP/DOCX/ODT: ZIP container parses, mandatory parts are present,
//!   slide / paragraph counts match the input
//! - HTML: standalone document contains slide sections, controls, and code gutters
//! - SVG/PNG: image-sequence output produces one file per slide

use std::io::Read;
use std::path::PathBuf;

fn assets() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn xml_attr_numbers(xml: &str, attr: &str) -> Vec<f32> {
    let needle = format!(r#"{attr}=""#);
    let mut values = Vec::new();
    let mut rest = xml;
    while let Some(pos) = rest.find(&needle) {
        let after = &rest[pos + needle.len()..];
        if let Some(end) = after.find('"') {
            let numeric = after[..end]
                .chars()
                .take_while(|ch| ch.is_ascii_digit() || matches!(ch, '.' | '-'))
                .collect::<String>();
            if let Ok(value) = numeric.parse::<f32>() {
                values.push(value);
            }
            rest = &after[end + 1..];
        } else {
            break;
        }
    }
    values
}

fn xml_element_attr_numbers(xml: &str, element: &str, attr: &str) -> Vec<f32> {
    let needle = format!("<{element}");
    let mut values = Vec::new();
    let mut rest = xml;
    while let Some(pos) = rest.find(&needle) {
        let after = &rest[pos..];
        if let Some(end) = after.find('>') {
            values.extend(xml_attr_numbers(&after[..end], attr));
            rest = &after[end + 1..];
        } else {
            break;
        }
    }
    values
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
            md2any::pdf::NotesPageSize::Slide,
            md2any::pdf::NotesLayout::Auto,
            None,
        )
        .unwrap(),
        "docx" => md2any::docx::write(&slides, &theme, "Renderer test", "tests", &base, None, None)
            .unwrap(),
        "odt" => md2any::odt::write(&slides, &theme, "Renderer test", "tests", &base, None, None)
            .unwrap(),
        "html" => md2any::html::write(
            &slides,
            &theme,
            &layout,
            "Renderer test",
            "tests",
            &base,
            None,
            None,
        )
        .unwrap(),
        other => panic!("unknown format {other}"),
    }
}

fn render_markdown(format: &str, md: &str) -> Vec<u8> {
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
        other => panic!("unsupported render_markdown format: {other}"),
    }
}

fn long_capability_table_md() -> &'static str {
    "---\n\
     title: Renderer test\n\
     theme: light\n\
     ---\n\
     # Deck\n\
     ## What md2any can do now\n\
     | Area | Capabilities |\n\
     |------|--------------|\n\
     | Outputs | PPTX, ODP, PDF, DOCX, ODT, standalone HTML, SVG slide images, PNG slide images |\n\
     | Slides | Section dividers, four deck layouts, light/dark themes, custom page sizes, transitions, TOC slides, presenter notes |\n\
     | Documents | Report-style DOCX/ODT with title page, contents, headers/footers, styled code/tables, image captions, notes appendices |\n\
     | Pagination | Smart continuation splitting for paragraphs, lists, code, columns, and tables; density control with `--break-fill` |\n\
     \n\
     The design goal is to keep the common deck/document job small enough to ship as one binary.\n"
}

fn render_pdf_from_md(md: &str, aspect: &str, layout_name: &str) -> Vec<u8> {
    let (front, body) = md2any::front_matter::extract(md);
    let theme = md2any::theme::Theme::resolve("light", aspect, None).unwrap();
    let layout = md2any::layout::Layout::resolve(layout_name).unwrap();
    let slides = md2any::parser::parse(&body, &front, "test");
    let slides = md2any::paginate::paginate_for_layout(slides, &theme, &layout);
    md2any::pdf::write(
        &slides,
        &theme,
        &layout,
        "Renderer test",
        "tests",
        &assets(),
        None,
        None,
        None,
        0.4,
        None,
        false,
        md2any::pdf::NotesPageSize::Slide,
        md2any::pdf::NotesLayout::Auto,
        None,
    )
    .unwrap()
}

fn render_notes_pdf(
    page_size: md2any::pdf::NotesPageSize,
    notes_layout: md2any::pdf::NotesLayout,
) -> Vec<u8> {
    let md = "---\n\
              title: Notes test\n\
              ---\n\
              # Deck\n\
              ## First slide\n\
              Visible content.\n\
              <!-- notes: Speaker-only detail. -->\n";
    let (front, body) = md2any::front_matter::extract(md);
    let theme = md2any::theme::Theme::resolve("light", "16:9", None).unwrap();
    let layout = md2any::layout::Layout::resolve("clean").unwrap();
    let slides = md2any::parser::parse(&body, &front, "test");
    let slides = md2any::paginate::paginate_for_layout(slides, &theme, &layout);
    md2any::pdf::write(
        &slides,
        &theme,
        &layout,
        "Notes test",
        "tests",
        &assets(),
        None,
        None,
        None,
        0.4,
        None,
        true,
        page_size,
        notes_layout,
        None,
    )
    .unwrap()
}

fn paginate_md_with_options(
    md: &str,
    aspect: &str,
    options: md2any::paginate::PaginationOptions,
) -> Vec<md2any::ir::Slide> {
    let (front, body) = md2any::front_matter::extract(md);
    let theme = md2any::theme::Theme::resolve("light", aspect, None).unwrap();
    let layout = md2any::layout::Layout::resolve("clean").unwrap();
    let slides = md2any::parser::parse(&body, &front, "test");
    md2any::paginate::paginate_for_layout_with_options(slides, &theme, &layout, options)
}

fn content_page_count(slides: &[md2any::ir::Slide]) -> usize {
    slides
        .iter()
        .filter(|s| matches!(s.kind, md2any::ir::SlideKind::Content))
        .count()
}

#[test]
fn table_column_alignment_is_captured() {
    let md = "---\n---\n## T\n| L | C | R |\n|:--|:-:|--:|\n| a | b | c |\n";
    let (front, body) = md2any::front_matter::extract(md);
    let slides = md2any::parser::parse(&body, &front, "test");
    let aligns = slides
        .iter()
        .flat_map(|s| &s.blocks)
        .find_map(|b| match b {
            md2any::ir::Block::Table { aligns, .. } => Some(aligns.clone()),
            _ => None,
        })
        .expect("table present");
    use md2any::ir::ColumnAlign::*;
    assert_eq!(
        aligns,
        vec![Left, Center, Right],
        "alignment from delimiter row"
    );
}

#[test]
fn tabs_in_code_expand_to_spaces() {
    let md = "---\n---\n## C\n```c\n\tx;\n```\n";
    let (front, body) = md2any::front_matter::extract(md);
    let slides = md2any::parser::parse(&body, &front, "test");
    let line = slides
        .iter()
        .flat_map(|s| &s.blocks)
        .find_map(|b| match b {
            md2any::ir::Block::CodeBlock { lines, .. } => lines.first().cloned(),
            _ => None,
        })
        .expect("code block present");
    assert!(!line.contains('\t'), "tabs should be expanded: {line:?}");
    assert!(line.starts_with("    x"), "tab -> 4 spaces: {line:?}");
}

#[test]
fn inline_svg_becomes_an_image_block() {
    let md = "---\n---\n## S\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\" height=\"10\"><rect width=\"10\" height=\"10\"/></svg>\n";
    let (front, body) = md2any::front_matter::extract(md);
    let slides = md2any::parser::parse(&body, &front, "test");
    let has_svg_image = slides.iter().flat_map(|s| &s.blocks).any(|b| {
        matches!(b, md2any::ir::Block::Image { src, .. } if src.starts_with("data:image/svg+xml"))
    });
    assert!(
        has_svg_image,
        "inline <svg> should become a data-URI image block"
    );
}

#[test]
fn oversize_image_width_is_clamped_not_leaked() {
    // `{width=150%}` must clamp to 100, not fall through and leak the literal
    // attribute text into the slide body.
    let md = "---\n---\n## Img\n![pic](examples/sample.png){width=150%}\n";
    let (front, body) = md2any::front_matter::extract(md);
    let slides = md2any::parser::parse(&body, &front, "test");
    let mut saw_image = false;
    for slide in &slides {
        for block in &slide.blocks {
            match block {
                md2any::ir::Block::Image { width_pct, .. } => {
                    saw_image = true;
                    assert_eq!(*width_pct, Some(100), "width should clamp to 100");
                }
                md2any::ir::Block::Paragraph(runs) => {
                    let text: String = runs.iter().map(|r| r.text.as_str()).collect();
                    assert!(
                        !text.contains("width="),
                        "literal width attribute leaked into body: {text:?}"
                    );
                }
                _ => {}
            }
        }
    }
    assert!(saw_image, "image block should be present");
}

#[test]
fn fence_form_columns_split_left_and_right() {
    // The fence form `::: … ::: … :::` (leading + trailing markers) must produce
    // a two-column block with content on BOTH sides, not dump everything right.
    let md = "---\n---\n## Cols\n\n:::\n\nLeft side.\n\n:::\n\nRight side.\n\n:::\n";
    let (front, body) = md2any::front_matter::extract(md);
    let theme = md2any::theme::Theme::resolve("light", "16:9", None).unwrap();
    let layout = md2any::layout::Layout::resolve("clean").unwrap();
    let slides = md2any::parser::parse(&body, &front, "test");
    let slides = md2any::paginate::paginate_for_layout_with_options(
        slides,
        &theme,
        &layout,
        md2any::paginate::PaginationOptions::default(),
    );
    let has_balanced_columns = slides.iter().flat_map(|s| &s.blocks).any(|b| {
        matches!(b, md2any::ir::Block::Columns { left, right } if !left.is_empty() && !right.is_empty())
    });
    assert!(
        has_balanced_columns,
        "fence-form columns should yield a Columns block with non-empty left and right"
    );
}

#[test]
fn display_math_equation_stays_on_its_heading_slide() {
    // A heading + short caption + one display equation must share a single
    // slide. Generated math images scale to the content width and render
    // short, so they must not carry the full-photo pagination weight that
    // would shove the equation onto a "(cont.)" overflow page.
    let md = "---\nmath: svg\n---\n\
## Bayes' Theorem\n\
How evidence updates belief.\n\n\
$$ P(A \\mid B) = \\frac{P(B \\mid A)\\,P(A)}{P(B)} $$\n";
    let (front, body) = md2any::front_matter::extract(md);
    let theme = md2any::theme::Theme::resolve("light", "16:9", None).unwrap();
    let layout = md2any::layout::Layout::resolve("clean").unwrap();
    let parse_opts = md2any::parser::ParseOptions {
        math_mode: md2any::math::MathMode::Svg,
        ..Default::default()
    };
    let slides = md2any::parser::parse_with_options(&body, &front, "test", parse_opts);
    let slides = md2any::paginate::paginate_for_layout_with_options(
        slides,
        &theme,
        &layout,
        md2any::paginate::PaginationOptions::default(),
    );
    assert_eq!(
        content_page_count(&slides),
        1,
        "equation should stay with its heading, not split to a (cont.) page"
    );
}

#[test]
fn render_plan_json_captures_layout_and_blocks() {
    let md = "---\ntitle: plan\n---\n# Deck\n## Diagnostics\nParagraph text for diagnostics.\n\n```rust\nfn main() {}\nprintln!(\"hi\");\nprintln!(\"bye\");\nprintln!(\"ok\");\nprintln!(\"done\");\nprintln!(\"end\");\n```\n";
    let (front, body) = md2any::front_matter::extract(md);
    let theme = md2any::theme::Theme::resolve("light", "16:9", None).unwrap();
    let layout = md2any::layout::Layout::resolve("clean").unwrap();
    let options = md2any::paginate::PaginationOptions::default();
    let slides = md2any::parser::parse(&body, &front, "plan");
    let slides =
        md2any::paginate::paginate_for_layout_with_options(slides, &theme, &layout, options);
    let plan = md2any::render_plan::build("plan.md", "plan", &slides, &theme, &layout, options);
    let json = serde_json::to_string(&plan).unwrap();
    let trace = md2any::render_plan::trace_text(&plan);
    let ir_json = serde_json::to_string(&md2any::render_plan::ir_dump(&slides)).unwrap();

    assert_eq!(plan.schema, "md2any-render-plan-v1");
    assert!(plan.layout.content_box.w_emu > 0);
    assert!(plan.pagination.budget_weight > 0.0);
    assert!(json.contains("\"kind\":\"code\""), "{json}");
    assert!(json.contains("\"start_line\":1"), "{json}");
    assert!(json.contains("\"table_fit\":\"auto\""), "{json}");
    assert!(trace.contains("md2any render plan"), "{trace}");
    assert!(trace.contains("table_fit=auto"), "{trace}");
    assert!(ir_json.contains("\"schema\":\"md2any-ir-v1\""), "{ir_json}");
}

#[test]
fn code_fence_include_loads_source_range_and_start_line() {
    let root = std::env::temp_dir().join("md2any-code-include-test");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("sample.rs"),
        "fn a() {}\nfn b() {}\nfn c() {}\nfn d() {}\nfn e() {}\nfn f() {}\n",
    )
    .unwrap();
    let md = "---\ntitle: include\n---\n# Deck\n## Snippet\n```rust file=sample.rs#L3-L5 title=\"core snippet\"\n```\n";
    let (front, body) = md2any::front_matter::extract(md);
    let slides = md2any::parser::parse_with_options(
        &body,
        &front,
        "include",
        md2any::parser::ParseOptions {
            include_base_dir: Some(root.clone()),
            ..Default::default()
        },
    );
    let block = slides
        .iter()
        .flat_map(|slide| slide.blocks.iter())
        .find_map(|block| match block {
            md2any::ir::Block::CodeBlock {
                lang,
                title,
                lines,
                start_line,
                ..
            } => Some((lang, title, lines, start_line)),
            _ => None,
        })
        .expect("expected included code block");

    assert_eq!(block.0.as_deref(), Some("rust"));
    assert_eq!(block.1.as_deref(), Some("core snippet"));
    assert_eq!(*block.3, 3);
    assert_eq!(
        block.2,
        &vec![
            "fn c() {}".to_string(),
            "fn d() {}".to_string(),
            "fn e() {}".to_string(),
        ]
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn cli_check_reports_failed_code_include() {
    let root = std::env::temp_dir().join("md2any-code-include-missing-test");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let input = root.join("deck.md");
    std::fs::write(
        &input,
        "---\ntitle: missing include\n---\n# Deck\n## Broken\n```rust file=missing.rs#L1-L2\n```\n",
    )
    .unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_md2any"))
        .arg(&input)
        .arg("--check")
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(2),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("code-include-failed"), "{stderr}");
    assert!(stderr.contains("missing.rs#L1-L2"), "{stderr}");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn math_source_mode_leaves_delimiters_untouched() {
    let md = "---\ntitle: math\n---\n# Deck\n## Formula\nInline $E = mc^2$ stays source.\n";
    let (front, body) = md2any::front_matter::extract(md);
    let slides = md2any::parser::parse_with_options(
        &body,
        &front,
        "math",
        md2any::parser::ParseOptions {
            math_mode: md2any::math::MathMode::Source,
            ..Default::default()
        },
    );
    let text = slides
        .iter()
        .flat_map(|slide| slide.blocks.iter())
        .find_map(|block| match block {
            md2any::ir::Block::Paragraph(runs) => Some(md2any::ir::runs_text(runs)),
            _ => None,
        })
        .unwrap();

    assert!(text.contains("$E = mc^2$"), "{text}");
}

#[test]
fn math_svg_mode_turns_display_math_into_data_image() {
    let md = "---\ntitle: math\n---\n# Deck\n## Formula\nInline $E = mc^2$ stays source.\n\n$$\\sum_{i=1}^{n} i = \\frac{n(n+1)}{2}$$\n";
    let (front, body) = md2any::front_matter::extract(md);
    let slides = md2any::parser::parse_with_options(
        &body,
        &front,
        "math",
        md2any::parser::ParseOptions {
            math_mode: md2any::math::MathMode::Svg,
            ..Default::default()
        },
    );
    let image_src = slides
        .iter()
        .flat_map(|slide| slide.blocks.iter())
        .find_map(|block| match block {
            md2any::ir::Block::Image { src, .. } => Some(src.as_str()),
            _ => None,
        })
        .expect("display math should become an image block");
    assert!(
        image_src.starts_with("data:image/svg+xml;base64,"),
        "{image_src}"
    );
    let meta = md2any::image::load_any(&assets(), image_src).unwrap();
    assert_eq!(meta.ext, "png");
    assert!(meta.width > 100 && meta.height > 20);

    let text = slides
        .iter()
        .flat_map(|slide| slide.blocks.iter())
        .filter_map(|block| match block {
            md2any::ir::Block::Paragraph(runs) => Some(md2any::ir::runs_text(runs)),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("Inline $E = mc^2$ stays source."), "{text}");
}

#[test]
fn math_unicode_supports_accents_text_and_alphabets() {
    let rendered = md2any::math::translate(
        "$\\vec{x} \\in \\mathbb{R}^n$, \
         $\\text{score} = \\hat{\\theta}$, \
         $\\argmax_x f(x)$, \
         $\\mathcal{F}$",
    );

    assert!(rendered.contains("x⃗"), "{rendered}");
    assert!(rendered.contains("ℝⁿ"), "{rendered}");
    assert!(rendered.contains("score"), "{rendered}");
    assert!(rendered.contains("θ̂"), "{rendered}");
    assert!(rendered.contains("arg maxₓ"), "{rendered}");
    assert!(rendered.contains("ℱ"), "{rendered}");
}

#[test]
fn math_front_matter_macros_apply_during_parse() {
    let md = r#"---
title: math
math_macros:
  '\RR': '\mathbb{R}'
---
# Deck
## Formula
Point $x \in \RR^n$.
"#;
    let (front, body) = md2any::front_matter::extract(md);
    assert_eq!(
        front.math_macros.as_ref().unwrap().get("\\RR"),
        Some(&"\\mathbb{R}".to_string())
    );
    let slides = md2any::parser::parse(&body, &front, "math");
    let text = slides
        .iter()
        .flat_map(|slide| slide.blocks.iter())
        .find_map(|block| match block {
            md2any::ir::Block::Paragraph(runs) => Some(md2any::ir::runs_text(runs)),
            _ => None,
        })
        .unwrap();

    assert!(text.contains("x ∈ℝⁿ"), "{text}");
}

#[test]
fn math_svg_mode_accepts_multiline_display_blocks() {
    let md = "---\ntitle: math\n---\n# Deck\n## Matrix\n\
$$\n\
\\begin{pmatrix}\n\
a & b \\\\\n\
c & d\n\
\\end{pmatrix}\n\
$$\n";
    let (front, body) = md2any::front_matter::extract(md);
    let slides = md2any::parser::parse_with_options(
        &body,
        &front,
        "math",
        md2any::parser::ParseOptions {
            math_mode: md2any::math::MathMode::Svg,
            ..Default::default()
        },
    );
    let image_srcs = slides
        .iter()
        .flat_map(|slide| slide.blocks.iter())
        .filter_map(|block| match block {
            md2any::ir::Block::Image { src, .. } => Some(src.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(image_srcs.len(), 1, "{image_srcs:?}");
    assert!(image_srcs[0].starts_with("data:image/svg+xml;base64,"));
    let meta = md2any::image::load_any(&assets(), image_srcs[0]).unwrap();
    assert_eq!(meta.ext, "png");
    assert!(meta.height > 80);

    let text = slides
        .iter()
        .flat_map(|slide| slide.blocks.iter())
        .filter_map(|block| match block {
            md2any::ir::Block::Paragraph(runs) => Some(md2any::ir::runs_text(runs)),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!text.contains("begin{pmatrix}"), "{text}");
}

#[test]
fn math_svg_display_images_render_at_natural_size() {
    let md = "---\ntitle: math\n---\n# Deck\n## Formula\n\
$$\\frac{-b \\pm \\sqrt{b^2 - 4ac}}{2a}$$\n";
    let (front, body) = md2any::front_matter::extract(md);
    let theme = md2any::theme::Theme::resolve("light", "16:9", None).unwrap();
    let layout = md2any::layout::Layout::resolve("clean").unwrap();
    let slides = md2any::parser::parse_with_options(
        &body,
        &front,
        "math",
        md2any::parser::ParseOptions {
            math_mode: md2any::math::MathMode::Svg,
            ..Default::default()
        },
    );
    let slides = md2any::paginate::paginate_for_layout(slides, &theme, &layout);

    let svg_files = md2any::svg::write_files(
        &slides,
        &theme,
        &layout,
        "math",
        "tests",
        &assets(),
        None,
        None,
        md2any::svg::ImageFormat::Svg,
    )
    .unwrap();
    let svg = String::from_utf8(svg_files.last().unwrap().bytes.clone()).unwrap();
    let image_heights = xml_element_attr_numbers(&svg, "svg", "height");
    let image_widths = xml_element_attr_numbers(&svg, "svg", "width");
    assert!(
        image_heights.iter().any(|h| *h <= 140.0),
        "math image should not be scaled to the full image slot: {image_heights:?}\n{svg}"
    );
    assert!(
        image_widths.iter().any(|w| *w <= 420.0),
        "math image should stay close to its natural width: {image_widths:?}\n{svg}"
    );

    let odp = md2any::odp::write(
        &slides,
        &theme,
        &layout,
        "math",
        "tests",
        &assets(),
        None,
        None,
        0.4,
        None,
    )
    .unwrap();
    let content = zip_read(&odp, "content.xml");
    let image_frame = content
        .split("<draw:frame")
        .find(|chunk| chunk.contains("<draw:image"))
        .expect("expected ODP image frame");
    let odp_heights_cm = xml_attr_numbers(image_frame, "svg:height");
    assert!(
        odp_heights_cm.iter().any(|h| *h <= 5.0),
        "ODP math image should stay close to natural size: {odp_heights_cm:?}"
    );
}

#[test]
fn math_svg_options_align_and_cap_generated_images() {
    let md = "---\ntitle: math\n---\n# Deck\n## Formula\n\
$$\\left(\\frac{a}{\\sqrt{b^2+c^2}}\\right)^2$$\n";
    let (front, body) = md2any::front_matter::extract(md);
    let theme = md2any::theme::Theme::resolve("light", "16:9", None).unwrap();
    let layout = md2any::layout::Layout::resolve("clean").unwrap();
    let slides = md2any::parser::parse_with_options(
        &body,
        &front,
        "math",
        md2any::parser::ParseOptions {
            math_mode: md2any::math::MathMode::Svg,
            math_svg: md2any::math::MathSvgOptions {
                scale_percent: 80,
                max_height_px: Some(72),
                block_align: md2any::math::MathBlockAlign::Left,
            },
            ..Default::default()
        },
    );
    let image = slides
        .iter()
        .flat_map(|slide| slide.blocks.iter())
        .find_map(|block| match block {
            md2any::ir::Block::Image { src, alt, .. } => Some((src.as_str(), alt.as_str())),
            _ => None,
        })
        .expect("display math should become an image");
    let meta = md2any::math::math_image_meta(image.0, image.1).expect("generated math metadata");
    assert_eq!(meta.align, md2any::math::MathBlockAlign::Left);
    assert_eq!(meta.max_height_px, Some(72));
    assert!(image.1.contains("scale=80"), "{}", image.1);

    let slides = md2any::paginate::paginate_for_layout(slides, &theme, &layout);
    let html = String::from_utf8(
        md2any::html::write(
            &slides,
            &theme,
            &layout,
            "math",
            "tests",
            &assets(),
            None,
            None,
        )
        .unwrap(),
    )
    .unwrap();
    assert!(html.contains("class=\"image math-image\""), "{html}");
    assert!(html.contains("--math-margin-left:0"), "{html}");
    assert!(html.contains("--math-max-height:72px"), "{html}");
    assert!(html.contains("alt=\"\""), "{html}");

    let svg_files = md2any::svg::write_files(
        &slides,
        &theme,
        &layout,
        "math",
        "tests",
        &assets(),
        None,
        None,
        md2any::svg::ImageFormat::Svg,
    )
    .unwrap();
    let svg = String::from_utf8(svg_files.last().unwrap().bytes.clone()).unwrap();
    let image_heights = xml_element_attr_numbers(&svg, "svg", "height");
    assert!(
        image_heights.iter().any(|h| *h <= 72.0),
        "SVG math image should honor the configured max height: {image_heights:?}\n{svg}"
    );

    let odp = md2any::odp::write(
        &slides,
        &theme,
        &layout,
        "math",
        "tests",
        &assets(),
        None,
        None,
        0.4,
        None,
    )
    .unwrap();
    let content = zip_read(&odp, "content.xml");
    let image_frame = content
        .split("<draw:frame")
        .find(|chunk| chunk.contains("<draw:image"))
        .expect("expected ODP image frame");
    let odp_heights_cm = xml_attr_numbers(image_frame, "svg:height");
    assert!(
        odp_heights_cm.iter().any(|h| *h <= 2.6),
        "ODP math image should honor the configured max height: {odp_heights_cm:?}"
    );
}

#[test]
fn rich_math_svg_fixture_renders_across_formats() {
    let md = "---\ntitle: rich math\n---\n# Deck\n## Native math\n\
$$\n\
\\left[\\begin{array}{cc}\n\
\\frac{1}{\\sqrt{x^2+1}} & y_i^2 \\\\\n\
\\alpha + \\beta & \\binom{n}{k}\n\
\\end{array}\\right]\n\
$$\n\
\n\
$$\n\
f(x)=\\begin{cases}\n\
x^2 & x \\ge 0 \\\\\n\
-x & x < 0\n\
\\end{cases}\n\
$$\n";
    let (front, body) = md2any::front_matter::extract(md);
    let theme = md2any::theme::Theme::resolve("light", "16:9", None).unwrap();
    let layout = md2any::layout::Layout::resolve("clean").unwrap();
    let slides = md2any::parser::parse_with_options(
        &body,
        &front,
        "rich math",
        md2any::parser::ParseOptions {
            math_mode: md2any::math::MathMode::Svg,
            ..Default::default()
        },
    );
    let slides = md2any::paginate::paginate_for_layout(slides, &theme, &layout);

    let pptx = md2any::pptx::write(
        &slides,
        &theme,
        &layout,
        "rich math",
        "tests",
        &assets(),
        None,
        None,
        0.4,
        None,
    )
    .unwrap();
    zip_contains(&pptx, &["ppt/presentation.xml", "ppt/media/image1.png"]).unwrap();

    let odp = md2any::odp::write(
        &slides,
        &theme,
        &layout,
        "rich math",
        "tests",
        &assets(),
        None,
        None,
        0.4,
        None,
    )
    .unwrap();
    zip_contains(&odp, &["content.xml"]).unwrap();
    assert!(zip_read(&odp, "content.xml").contains("<draw:image"));

    let pdf = md2any::pdf::write(
        &slides,
        &theme,
        &layout,
        "rich math",
        "tests",
        &assets(),
        None,
        None,
        None,
        0.4,
        None,
        false,
        md2any::pdf::NotesPageSize::Slide,
        md2any::pdf::NotesLayout::Auto,
        None,
    )
    .unwrap();
    assert!(pdf.starts_with(b"%PDF-"));

    let docx =
        md2any::docx::write(&slides, &theme, "rich math", "tests", &assets(), None, None).unwrap();
    zip_contains(&docx, &["word/document.xml", "word/media/image1.png"]).unwrap();
    assert!(!zip_read(&docx, "word/document.xml").contains("math;scale="));

    let odt =
        md2any::odt::write(&slides, &theme, "rich math", "tests", &assets(), None, None).unwrap();
    zip_contains(&odt, &["content.xml", "Pictures/image1.png"]).unwrap();
    assert!(!zip_read(&odt, "content.xml").contains("math;scale="));

    let html = md2any::html::write(
        &slides,
        &theme,
        &layout,
        "rich math",
        "tests",
        &assets(),
        None,
        None,
    )
    .unwrap();
    assert!(String::from_utf8(html).unwrap().contains("math-image"));

    let svg_files = md2any::svg::write_files(
        &slides,
        &theme,
        &layout,
        "rich math",
        "tests",
        &assets(),
        None,
        None,
        md2any::svg::ImageFormat::Svg,
    )
    .unwrap();
    assert!(svg_files
        .iter()
        .any(|file| String::from_utf8_lossy(&file.bytes).contains("overflow=\"visible\"")));

    let png_files = md2any::svg::write_files(
        &slides,
        &theme,
        &layout,
        "rich math",
        "tests",
        &assets(),
        None,
        None,
        md2any::svg::ImageFormat::Png,
    )
    .unwrap();
    assert!(png_files
        .iter()
        .any(|file| file.bytes.starts_with(b"\x89PNG")));
}

#[test]
fn standard_model_lagrangian_a4_uses_selectable_math_layout() {
    let deck_path = assets().join("examples/standard-model-lagrangian-a4.md");
    let md = std::fs::read_to_string(&deck_path).unwrap();
    let (front, body) = md2any::front_matter::extract(&md);
    let theme = md2any::theme::Theme::resolve(
        front.theme.as_deref().unwrap_or("light"),
        front.aspect.as_deref().unwrap_or("16:9"),
        None,
    )
    .unwrap();
    let layout =
        md2any::layout::Layout::resolve(front.layout.as_deref().unwrap_or("clean")).unwrap();
    let example_dir = deck_path.parent().unwrap().to_path_buf();
    let slides = md2any::parser::parse_with_options(
        &body,
        &front,
        "Standard Model Lagrangian Markup A4",
        md2any::parser::ParseOptions {
            include_base_dir: Some(example_dir.clone()),
            ..Default::default()
        },
    );
    let slides = md2any::paginate::paginate_for_layout_with_options(
        slides,
        &theme,
        &layout,
        md2any::paginate::PaginationOptions {
            break_mode: md2any::paginate::BreakMode::Off,
            ..Default::default()
        },
    );

    assert_eq!(slides.len(), 1, "markup page should stay one slide");
    let Some((lines, lang)) = slides[0].full_page_code() else {
        panic!(
            "expected text-full slide with exactly one math block: {:?}",
            slides[0]
        );
    };
    assert!(md2any::math::is_markup_text_language(lang));
    assert!(
        lines.len() >= 70,
        "expected dense formula lines: {}",
        lines.len()
    );

    let probe = md2any::math::layout_markup_text(r"\frac{1}{\sqrt{x^2+1}}y_i^2", 100);
    assert!(probe
        .draws
        .iter()
        .any(|draw| { matches!(draw, md2any::math::MathLayoutDraw::Line { .. }) }));
    assert!(probe
        .draws
        .iter()
        .any(|draw| { matches!(draw, md2any::math::MathLayoutDraw::Polyline { .. }) }));

    let pdf = md2any::pdf::write(
        &slides,
        &theme,
        &layout,
        "Standard Model Lagrangian Markup A4",
        "tests",
        &example_dir,
        None,
        None,
        None,
        0.4,
        None,
        false,
        md2any::pdf::NotesPageSize::Slide,
        md2any::pdf::NotesLayout::Auto,
        None,
    )
    .unwrap();
    assert_eq!(pdf_media_boxes(&pdf), vec![(595, 842)]);
    let pdf_text = String::from_utf8_lossy(&pdf);
    assert!(
        !pdf_text.contains("/Subtype /Image"),
        "markup math should render as selectable text and vector strokes"
    );
    assert!(
        pdf_text.contains("/ToUnicode"),
        "PDF text should keep copy/paste maps"
    );
    let positions = pdf_text_positions(&pdf);
    assert!(
        positions.len() > lines.len() * 2,
        "positioned math should emit multiple selectable text runs per formula line, got {} for {} lines",
        positions.len(),
        lines.len()
    );
    let streams = pdf_flate_streams(&pdf).join("\n");
    assert!(
        streams.contains(" l\nS") || streams.contains(" c\nS"),
        "math layout should draw vector fraction/radical/delimiter strokes"
    );

    let svg_files = md2any::svg::write_files(
        &slides,
        &theme,
        &layout,
        "Standard Model Lagrangian Markup A4",
        "tests",
        &example_dir,
        None,
        None,
        md2any::svg::ImageFormat::Svg,
    )
    .unwrap();
    assert_eq!(svg_files.len(), 1);
    let svg = String::from_utf8(svg_files[0].bytes.clone()).unwrap();
    assert!(svg.contains("<line "), "{svg}");
    assert!(svg.contains("<text "), "{svg}");
    assert!(!svg.contains("\\frac"), "{svg}");
    assert!(!svg.contains("^{"), "{svg}");
}

#[test]
fn math_diagnostics_catch_unsupported_constructs_and_skip_code() {
    let md = "Inline $\\foo{x}$ and supported $\\vec{x}$.\n\
              $$\\begin{matrix} a & b \\\\ c & d \\end{matrix}$$\n\
              `code $\\foo{y}$`\n\
              ```\n\
              $\\foo{z}$\n\
              ```\n";
    let diagnostics = md2any::math::diagnose(md);

    assert!(
        diagnostics
            .iter()
            .any(|d| d.kind == "unsupported-math-macro" && d.detail.contains("\\foo")),
        "{diagnostics:?}"
    );
    assert!(
        diagnostics
            .iter()
            .any(|d| d.kind == "unsupported-math-environment"),
        "{diagnostics:?}"
    );
    assert!(
        diagnostics
            .iter()
            .any(|d| d.kind == "unsupported-math-linebreak"),
        "{diagnostics:?}"
    );
    assert_eq!(
        diagnostics
            .iter()
            .filter(|d| d.detail.contains("\\foo"))
            .count(),
        1,
        "inline/fenced code math should be ignored: {diagnostics:?}"
    );
    assert!(
        !diagnostics.iter().any(|d| d.detail.contains("\\vec")),
        "supported accent should not warn: {diagnostics:?}"
    );
}

#[test]
fn font_audit_reports_missing_visible_glyphs() {
    let marker = char::from_u32(0x10FFFF).unwrap();
    let md = format!("---\ntitle: fonts\n---\n# Deck\n## Glyphs\nMissing glyph: {marker}\n");
    let (front, body) = md2any::front_matter::extract(&md);
    let slides = md2any::parser::parse(&body, &front, "fonts");
    let fonts = md2any::font::PdfFonts::load(None).unwrap();
    let audit = md2any::font::audit_pdf_fonts(&slides, &fonts);

    assert!(
        audit.missing.iter().any(|hit| hit.codepoint == "U+10FFFF"),
        "expected U+10FFFF missing glyph, got {:?}",
        audit.missing
    );
}

#[test]
fn pdf_custom_font_option_replaces_sans_face() {
    let md = "---\ntitle: fonts\n---\n# Deck\n## Custom PDF font\nText.\n";
    let (front, body) = md2any::front_matter::extract(md);
    let theme = md2any::theme::Theme::resolve("light", "16:9", None).unwrap();
    let layout = md2any::layout::Layout::resolve("clean").unwrap();
    let slides = md2any::parser::parse(&body, &front, "fonts");
    let slides = md2any::paginate::paginate_for_layout(slides, &theme, &layout);
    let pdf_font = assets().join("assets/fonts/DejaVuSans.ttf");
    let bytes = md2any::pdf::write_with_font_options(
        &slides,
        &theme,
        &layout,
        "fonts",
        "tests",
        &assets(),
        None,
        None,
        None,
        0.4,
        None,
        false,
        md2any::pdf::NotesPageSize::Slide,
        md2any::pdf::NotesLayout::Auto,
        md2any::font::PdfFontOptions {
            pdf_font: Some(pdf_font.as_path()),
            ..Default::default()
        },
    )
    .unwrap();
    let full = String::from_utf8_lossy(&bytes);

    assert!(
        full.contains("CustomSans"),
        "expected custom PDF font name in output"
    );
}

#[test]
fn cli_speaker_package_writes_artifacts_and_manifest() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "md2any-speaker-package-{unique}-{}",
        std::process::id()
    ));
    let input = root.join("talk deck.md");
    let package_dir = root.join("release");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        &input,
        "---\ntitle: Package test\n---\n# Deck\n## Slide one\nVisible content.\n<!-- notes: Speaker note. -->\n",
    )
    .unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_md2any"))
        .arg(&input)
        .arg("--speaker-package")
        .arg(&package_dir)
        .arg("--format")
        .arg("odp")
        .arg("--handout")
        .arg("2")
        .arg("--quiet")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "speaker package command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let deck = package_dir.join("talk-deck.odp");
    let notes = package_dir.join("talk-deck-notes.pdf");
    let handout = package_dir.join("talk-deck-handout.pdf");
    let manifest = package_dir.join("talk-deck-manifest.json");
    assert!(deck.exists(), "missing {}", deck.display());
    assert!(notes.exists(), "missing {}", notes.display());
    assert!(handout.exists(), "missing {}", handout.display());
    assert!(manifest.exists(), "missing {}", manifest.display());

    let manifest_text = std::fs::read_to_string(&manifest).unwrap();
    assert!(
        manifest_text.contains("\"schema\": \"md2any-speaker-package-v1\""),
        "{manifest_text}"
    );
    assert!(
        manifest_text.contains("\"deck_format\": \"odp\""),
        "{manifest_text}"
    );
    assert!(
        manifest_text.contains("\"handout_slides_per_page\": 2"),
        "{manifest_text}"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn cli_font_audit_exits_two_for_missing_glyphs() {
    let marker = char::from_u32(0x10FFFF).unwrap();
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("md2any-font-audit-{unique}-{}", std::process::id()));
    let input = root.join("talk.md");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        &input,
        format!("---\ntitle: Audit\n---\n# Deck\n## Slide\nMissing: {marker}\n"),
    )
    .unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_md2any"))
        .arg(&input)
        .arg("--font-audit")
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(2),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("U+10FFFF"),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn cli_check_reports_math_diagnostics() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("md2any-math-check-{unique}-{}", std::process::id()));
    let input = root.join("talk.md");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        &input,
        "---\ntitle: Math\n---\n# Deck\n## Formula\n$$\\begin{matrix} a & b \\\\ c & d \\end{matrix}$$\n",
    )
    .unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_md2any"))
        .arg(&input)
        .arg("--check")
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(2),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("md2any math check"), "{stderr}");
    assert!(stderr.contains("unsupported-math-environment"), "{stderr}");

    let _ = std::fs::remove_dir_all(root);
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

fn find_bytes(haystack: &[u8], needle: &[u8], start: usize) -> Option<usize> {
    haystack[start..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| start + p)
}

fn pdf_flate_streams(bytes: &[u8]) -> Vec<String> {
    let mut streams = Vec::new();
    let mut pos = 0;
    while let Some(stream_pos) = find_bytes(bytes, b"stream\n", pos) {
        let data_start = stream_pos + b"stream\n".len();
        let Some(data_end) = find_bytes(bytes, b"\nendstream", data_start) else {
            break;
        };
        let mut decoder = flate2::read::ZlibDecoder::new(&bytes[data_start..data_end]);
        let mut decoded = String::new();
        if decoder.read_to_string(&mut decoded).is_ok() {
            streams.push(decoded);
        }
        pos = data_end + b"\nendstream".len();
    }
    streams
}

fn pdf_text_positions(bytes: &[u8]) -> Vec<(f32, f32)> {
    let mut positions = Vec::new();
    for stream in pdf_flate_streams(bytes) {
        for line in stream.lines() {
            let trimmed = line.trim();
            if !trimmed.ends_with(" Td") {
                continue;
            }
            let mut parts = trimmed.split_whitespace();
            let (Some(x), Some(y), Some("Td")) = (parts.next(), parts.next(), parts.next()) else {
                continue;
            };
            if let (Ok(x), Ok(y)) = (x.parse::<f32>(), y.parse::<f32>()) {
                positions.push((x, y));
            }
        }
    }
    positions
}

fn pdf_media_boxes(bytes: &[u8]) -> Vec<(u32, u32)> {
    let text = String::from_utf8_lossy(bytes);
    text.split("/MediaBox [0 0 ")
        .skip(1)
        .filter_map(|part| {
            let mut nums = part.split(']').next()?.split_whitespace();
            let w = nums.next()?.parse::<u32>().ok()?;
            let h = nums.next()?.parse::<u32>().ok()?;
            Some((w, h))
        })
        .collect()
}

fn pdf_divider_rects(bytes: &[u8]) -> Vec<(f32, f32)> {
    let mut rects = Vec::new();
    for stream in pdf_flate_streams(bytes) {
        let mut after_notes_divider_color = false;
        for line in stream.lines() {
            let trimmed = line.trim();
            if trimmed == "0.85 0.89 0.94 rg" {
                after_notes_divider_color = true;
                continue;
            }
            if !after_notes_divider_color {
                continue;
            }
            after_notes_divider_color = false;
            let mut parts = trimmed.split_whitespace();
            let (Some(_x), Some(_y), Some(w), Some(h), Some("re"), Some("f")) = (
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
            ) else {
                continue;
            };
            if let (Ok(w), Ok(h)) = (w.parse::<f32>(), h.parse::<f32>()) {
                rects.push((w, h));
            }
        }
    }
    rects
}

fn pdf_rects_after_color(bytes: &[u8], color_cmd: &str) -> Vec<(f32, f32)> {
    let mut rects = Vec::new();
    for stream in pdf_flate_streams(bytes) {
        let mut after_color = false;
        for line in stream.lines() {
            let trimmed = line.trim();
            if trimmed == color_cmd {
                after_color = true;
                continue;
            }
            if !after_color {
                continue;
            }
            after_color = false;
            let mut parts = trimmed.split_whitespace();
            let (Some(_x), Some(_y), Some(w), Some(h), Some("re"), Some("f")) = (
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
            ) else {
                continue;
            };
            if let (Ok(w), Ok(h)) = (w.parse::<f32>(), h.parse::<f32>()) {
                rects.push((w, h));
            }
        }
    }
    rects
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

#[test]
fn pdf_landscape_long_list_uses_second_column() {
    let mut md = String::from("---\ntitle: lists\n---\n# Deck\n## Dense bullets\n");
    for i in 1..=24 {
        md.push_str(&format!("- item {i:02}\n"));
    }

    let bytes = render_pdf_from_md(&md, "16:9", "clean");
    let positions = pdf_text_positions(&bytes);
    let right_column_items = positions
        .iter()
        .filter(|(x, y)| *x > 460.0 && *y > 80.0 && *y < 430.0)
        .count();

    assert!(
        right_column_items >= 8,
        "expected list text in the right column, got positions: {positions:?}"
    );
}

#[test]
fn pdf_notes_pages_use_slide_size_by_default() {
    let bytes = render_notes_pdf(
        md2any::pdf::NotesPageSize::Slide,
        md2any::pdf::NotesLayout::Auto,
    );
    let boxes = pdf_media_boxes(&bytes);

    assert!(
        boxes.iter().any(|b| *b == (960, 540)),
        "expected 16:9 notes pages by default, got {boxes:?}"
    );
    assert!(
        !boxes.iter().any(|b| *b == (595, 842)),
        "notes pages should not default to A4 portrait, got {boxes:?}"
    );
}

#[test]
fn pdf_notes_pages_can_use_a4_for_printing() {
    let bytes = render_notes_pdf(
        md2any::pdf::NotesPageSize::A4,
        md2any::pdf::NotesLayout::Auto,
    );
    let boxes = pdf_media_boxes(&bytes);

    assert!(
        boxes.iter().any(|b| *b == (595, 842)),
        "expected explicit A4 notes pages, got {boxes:?}"
    );
}

#[test]
fn pdf_notes_layout_can_force_below_on_landscape() {
    let bytes = render_notes_pdf(
        md2any::pdf::NotesPageSize::Slide,
        md2any::pdf::NotesLayout::Below,
    );
    let rects = pdf_divider_rects(&bytes);

    assert!(
        rects.iter().any(|(w, h)| *w > 300.0 && *h <= 1.0),
        "expected a horizontal notes divider for below layout, got {rects:?}"
    );
    assert!(
        !rects.iter().any(|(w, h)| *w <= 1.0 && *h > 200.0),
        "below layout should not use a vertical notes divider, got {rects:?}"
    );
}

#[test]
fn paginate_portrait_long_list_splits_single_column() {
    let mut md = String::from("---\ntitle: lists\n---\n# Deck\n## Portrait bullets\n");
    for i in 1..=26 {
        md.push_str(&format!("- item {i:02}\n"));
    }

    let (front, body) = md2any::front_matter::extract(&md);
    let theme = md2any::theme::Theme::resolve("light", "a5", None).unwrap();
    let layout = md2any::layout::Layout::resolve("clean").unwrap();
    let slides = md2any::parser::parse(&body, &front, "test");
    let slides = md2any::paginate::paginate_for_layout(slides, &theme, &layout);
    let content_pages = slides
        .iter()
        .filter(|s| matches!(s.kind, md2any::ir::SlideKind::Content))
        .count();

    assert!(
        content_pages >= 2,
        "portrait long list should be split, got {content_pages} content page(s)"
    );
}

#[test]
fn smart_break_splits_long_paragraph_at_sentence_boundary() {
    let mut paragraph = String::new();
    for i in 1..=26 {
        paragraph.push_str(&format!(
            "Sentence {i:02} keeps enough words together for readable paragraph pagination. "
        ));
    }
    let md = format!("---\ntitle: smart\n---\n# Deck\n## Long paragraph\n{paragraph}\n");
    let slides = paginate_md_with_options(
        &md,
        "a5",
        md2any::paginate::PaginationOptions {
            break_mode: md2any::paginate::BreakMode::Smart,
            fill: 1.0,
            ..Default::default()
        },
    );

    assert!(
        content_page_count(&slides) >= 2,
        "smart mode should split an overlong paragraph"
    );
    let first_para = slides
        .iter()
        .find(|s| matches!(s.kind, md2any::ir::SlideKind::Content))
        .and_then(|s| s.blocks.first())
        .and_then(|b| match b {
            md2any::ir::Block::Paragraph(runs) => Some(md2any::ir::runs_text(runs)),
            _ => None,
        })
        .expect("first content block should be a paragraph");
    assert!(
        first_para.ends_with('.'),
        "smart paragraph split should prefer sentence boundaries: {first_para:?}"
    );
    assert!(
        first_para.len() < paragraph.len(),
        "first chunk should not contain the whole paragraph"
    );
}

#[test]
fn simple_break_keeps_long_paragraph_atomic() {
    let mut paragraph = String::new();
    for i in 1..=26 {
        paragraph.push_str(&format!(
            "Sentence {i:02} keeps enough words together for readable paragraph pagination. "
        ));
    }
    let md = format!("---\ntitle: simple\n---\n# Deck\n## Long paragraph\n{paragraph}\n");
    let slides = paginate_md_with_options(
        &md,
        "a5",
        md2any::paginate::PaginationOptions {
            break_mode: md2any::paginate::BreakMode::Simple,
            fill: 1.0,
            ..Default::default()
        },
    );

    assert_eq!(
        content_page_count(&slides),
        1,
        "simple mode should leave paragraphs as one block"
    );
}

#[test]
fn break_fill_controls_list_density() {
    let mut md = String::from("---\ntitle: density\n---\n# Deck\n## Dense bullets\n");
    for i in 1..=42 {
        md.push_str(&format!("- item {i:02}\n"));
    }

    let airy = paginate_md_with_options(
        &md,
        "a5",
        md2any::paginate::PaginationOptions {
            break_mode: md2any::paginate::BreakMode::Smart,
            fill: 0.5,
            ..Default::default()
        },
    );
    let dense = paginate_md_with_options(
        &md,
        "a5",
        md2any::paginate::PaginationOptions {
            break_mode: md2any::paginate::BreakMode::Smart,
            fill: 1.2,
            ..Default::default()
        },
    );

    assert!(
        content_page_count(&airy) > content_page_count(&dense),
        "lower fill should produce more continuation slides: airy={}, dense={}",
        content_page_count(&airy),
        content_page_count(&dense)
    );
}

#[test]
fn table_fit_split_repeats_key_column_for_wide_tables() {
    let md = "---\ntitle: tables\n---\n# Deck\n## Wide table\n\
| Key | A | B | C | D | E | F | G | H |\n\
|-----|---|---|---|---|---|---|---|---|\n\
| r1  | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 |\n\
| r2  | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 |\n";
    let slides = paginate_md_with_options(
        md,
        "16:9",
        md2any::paginate::PaginationOptions {
            break_mode: md2any::paginate::BreakMode::Smart,
            fill: 1.0,
            table_fit: md2any::paginate::TableFit::Split,
            ..Default::default()
        },
    );
    let tables = slides
        .iter()
        .flat_map(|slide| slide.blocks.iter())
        .filter_map(|block| match block {
            md2any::ir::Block::Table { headers, rows, .. } => Some((headers, rows)),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert!(
        tables.len() >= 2,
        "wide table should split into column groups: {tables:?}"
    );
    assert_eq!(md2any::ir::runs_text(&tables[0].0[0]), "Key");
    assert_eq!(md2any::ir::runs_text(&tables[1].0[0]), "Key");
    assert!(tables[0].0.len() <= 7, "first chunk too wide");
    assert!(tables[1].0.len() <= 7, "second chunk too wide");
    assert_eq!(md2any::ir::runs_text(&tables[1].1[0][0]), "r1");
}

#[test]
fn table_fit_auto_transposes_compact_portrait_tables() {
    let md = "---\ntitle: tables\n---\n# Deck\n## Portrait table\n\
| Key | A | B | C | D | E |\n\
|-----|---|---|---|---|---|\n\
| r1  | 1 | 2 | 3 | 4 | 5 |\n\
| r2  | 6 | 7 | 8 | 9 | 10 |\n";
    let slides =
        paginate_md_with_options(md, "9:16", md2any::paginate::PaginationOptions::default());
    let (headers, rows) = slides
        .iter()
        .flat_map(|slide| slide.blocks.iter())
        .find_map(|block| match block {
            md2any::ir::Block::Table { headers, rows, .. } => Some((headers, rows)),
            _ => None,
        })
        .expect("expected transposed table");

    let header_text = headers
        .iter()
        .map(|runs| md2any::ir::runs_text(runs))
        .collect::<Vec<_>>();
    assert_eq!(header_text, vec!["Field", "Row 1", "Row 2"]);
    assert_eq!(rows.len(), 6);
    assert_eq!(md2any::ir::runs_text(&rows[0][0]), "Key");
    assert_eq!(md2any::ir::runs_text(&rows[0][1]), "r1");
    assert_eq!(md2any::ir::runs_text(&rows[0][2]), "r2");
}

#[test]
fn smart_break_splits_wrapped_table_rows_by_estimated_height() {
    let mut md = String::from(
        "---\ntitle: tables\n---\n# Deck\n## Wrapped capability table\n\
| Area | Capabilities |\n\
|------|--------------|\n",
    );
    for i in 1..=8 {
        md.push_str(&format!(
            "| Row {i} | This row has enough descriptive text to wrap over multiple \
visual lines in the rendered table, so it should consume more than one fixed \
row of pagination budget. |\n"
        ));
    }

    let slides = paginate_md_with_options(
        &md,
        "16:9",
        md2any::paginate::PaginationOptions {
            break_mode: md2any::paginate::BreakMode::Smart,
            fill: 1.0,
            table_fit: md2any::paginate::TableFit::Auto,
            ..Default::default()
        },
    );
    let tables = slides
        .iter()
        .flat_map(|slide| slide.blocks.iter())
        .filter_map(|block| match block {
            md2any::ir::Block::Table { rows, .. } => Some(rows.len()),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert!(
        tables.len() >= 2,
        "wrapped table should split across slides: {tables:?}"
    );
    assert_eq!(tables.iter().sum::<usize>(), 8);
    assert!(
        tables.iter().all(|rows| *rows <= 4),
        "wrapped row chunks should leave footer breathing room: {tables:?}"
    );
}

#[test]
fn code_block_line_numbers_continue_after_split() {
    let mut md = String::from("---\ntitle: code\n---\n# Deck\n## Long code\n```rust\n");
    for _ in 1..=36 {
        md.push_str("let value = compute_value();\n");
    }
    md.push_str("```\n");

    let (front, body) = md2any::front_matter::extract(&md);
    let theme = md2any::theme::Theme::resolve("light", "a5", None).unwrap();
    let layout = md2any::layout::Layout::resolve("clean").unwrap();
    let parsed = md2any::parser::parse(&body, &front, "test");
    let slides = md2any::paginate::paginate_for_layout_with_options(
        parsed,
        &theme,
        &layout,
        md2any::paginate::PaginationOptions {
            break_mode: md2any::paginate::BreakMode::Smart,
            fill: 1.0,
            ..Default::default()
        },
    );

    let code_chunks: Vec<(usize, usize, usize)> = slides
        .iter()
        .enumerate()
        .filter_map(|(slide_idx, slide)| {
            slide.blocks.iter().find_map(|block| match block {
                md2any::ir::Block::CodeBlock {
                    lines, start_line, ..
                } => Some((slide_idx, *start_line, lines.len())),
                _ => None,
            })
        })
        .collect();

    assert!(
        code_chunks.len() >= 2,
        "expected long code to split into chunks, got {code_chunks:?}"
    );

    let mut expected_start = 1;
    for (_, start_line, len) in &code_chunks {
        assert_eq!(
            *start_line, expected_start,
            "code chunks should preserve source line offsets"
        );
        expected_start += *len;
    }

    let bytes = md2any::pptx::write(
        &slides,
        &theme,
        &layout,
        "Renderer test",
        "tests",
        &assets(),
        None,
        None,
        0.4,
        None,
    )
    .unwrap();
    let (slide_idx, start_line, len) = code_chunks
        .iter()
        .copied()
        .find(|(_, start_line, _)| *start_line > 1)
        .expect("expected a continuation code chunk");
    let xml = zip_read(&bytes, &format!("ppt/slides/slide{}.xml", slide_idx + 1));
    let last_line = start_line + len - 1;
    let width = last_line.to_string().len();
    let expected_gutter = format!("<a:t>{:>width$}  </a:t>", start_line, width = width);
    assert!(
        xml.contains(&expected_gutter),
        "continuation slide should start gutter at source line {start_line}; expected {expected_gutter:?} in {xml}"
    );
}

#[test]
fn language_tag_headers_count_toward_code_pagination() {
    let md = r#"---
title: code
---
# Deck
## Tagged snippets
```haskell
loop :: IO ()
loop = do
  print (0 == 0)
  loop
```

```bcpl
GET "LIBHDR"

LET start() BE
$(
  FOR i = 1 TO 3 DO writes("BCPL still has opinions!*N")
  RESULTIS 0
$)
```

```bf
+++++[>+++++++<-]>.
```
"#;

    let (front, body) = md2any::front_matter::extract(md);
    let theme = md2any::theme::Theme::resolve("light", "16:9", None).unwrap();
    let layout = md2any::layout::Layout::resolve("clean").unwrap();
    let slides = md2any::paginate::paginate_for_layout_with_options(
        md2any::parser::parse(&body, &front, "test"),
        &theme,
        &layout,
        md2any::paginate::PaginationOptions::default(),
    );

    let tagged_shapes: Vec<(usize, usize)> = slides
        .iter()
        .filter(|slide| slide.title.starts_with("Tagged snippets"))
        .map(|slide| {
            let code = slide
                .blocks
                .iter()
                .filter(|block| matches!(block, md2any::ir::Block::CodeBlock { .. }))
                .count();
            let paragraphs = slide
                .blocks
                .iter()
                .filter(|block| matches!(block, md2any::ir::Block::Paragraph(_)))
                .count();
            (code, paragraphs)
        })
        .collect();

    assert_eq!(
        tagged_shapes,
        vec![(2, 0), (1, 0)],
        "tagged code headers should count toward slide height so code does not collide with the footer"
    );
}

#[test]
fn code_columns_two_up_preserves_source_line_numbers() {
    let mut md = String::from(
        "---\ntitle: code\n---\n# Deck\n## Two-up code\n```rust columns=2 start=10 title=\"method\"\n",
    );
    for i in 0..20 {
        md.push_str(&format!("let value_{i} = compute({i});\n"));
    }
    md.push_str("```\n");

    let (front, body) = md2any::front_matter::extract(&md);
    let theme = md2any::theme::Theme::resolve("light", "16:9", None).unwrap();
    let layout = md2any::layout::Layout::resolve("clean").unwrap();
    let parsed = md2any::parser::parse(&body, &front, "test");
    let slides = md2any::paginate::paginate_for_layout_with_options(
        parsed,
        &theme,
        &layout,
        md2any::paginate::PaginationOptions::default(),
    );

    let columns = slides
        .iter()
        .flat_map(|slide| slide.blocks.iter())
        .find_map(|block| match block {
            md2any::ir::Block::Columns { left, right } => Some((left, right)),
            _ => None,
        })
        .expect("expected code block to become two-up columns");

    let left_code = match &columns.0[0] {
        md2any::ir::Block::CodeBlock {
            lines, start_line, ..
        } => (*start_line, lines.len()),
        other => panic!("expected left code block, got {other:?}"),
    };
    let right_code = match &columns.1[0] {
        md2any::ir::Block::CodeBlock {
            lines, start_line, ..
        } => (*start_line, lines.len()),
        other => panic!("expected right code block, got {other:?}"),
    };

    assert_eq!(left_code, (10, 10));
    assert_eq!(right_code, (20, 10));
}

#[test]
fn code_columns_global_two_up_is_landscape_only() {
    let mut md = String::from("---\ntitle: code\n---\n# Deck\n## Code\n```rust\n");
    for i in 0..18 {
        md.push_str(&format!("let value_{i} = compute({i});\n"));
    }
    md.push_str("```\n");

    let (front, body) = md2any::front_matter::extract(&md);
    let layout = md2any::layout::Layout::resolve("clean").unwrap();
    let options = md2any::paginate::PaginationOptions {
        code_columns: md2any::ir::CodeColumns::TwoUp,
        ..Default::default()
    };

    let landscape = md2any::theme::Theme::resolve("light", "16:9", None).unwrap();
    let landscape_slides = md2any::paginate::paginate_for_layout_with_options(
        md2any::parser::parse(&body, &front, "test"),
        &landscape,
        &layout,
        options,
    );
    assert!(
        landscape_slides
            .iter()
            .flat_map(|slide| slide.blocks.iter())
            .any(|block| matches!(block, md2any::ir::Block::Columns { .. })),
        "landscape global two-up should produce a columns block"
    );

    let portrait = md2any::theme::Theme::resolve("light", "9:16", None).unwrap();
    let portrait_slides = md2any::paginate::paginate_for_layout_with_options(
        md2any::parser::parse(&body, &front, "test"),
        &portrait,
        &layout,
        options,
    );
    assert!(
        portrait_slides
            .iter()
            .flat_map(|slide| slide.blocks.iter())
            .all(|block| !matches!(block, md2any::ir::Block::Columns { .. })),
        "portrait global two-up should fall back to single column"
    );
}

// ---------------------------------------------------------------------------
// HTML
// ---------------------------------------------------------------------------

#[test]
fn html_contains_standalone_deck_shell() {
    let bytes = render("html");
    let html = String::from_utf8(bytes).unwrap();

    assert!(html.starts_with("<!doctype html>"), "{html}");
    assert!(html.contains("md2any-html-v1"), "{html}");
    assert!(
        html.contains("<section class=\"slide slide-title active\""),
        "{html}"
    );
    assert!(html.contains("data-slide=\"1\""), "{html}");
    assert!(html.contains("data-next"), "{html}");
    assert!(html.contains("function show(next)"), "{html}");
    assert!(html.contains("https://example.com"), "{html}");
}

#[test]
fn html_code_gutter_uses_block_start_line() {
    let theme = md2any::theme::Theme::resolve("light", "16:9", None).unwrap();
    let layout = md2any::layout::Layout::resolve("clean").unwrap();
    let slides = vec![md2any::ir::Slide {
        kind: md2any::ir::SlideKind::Content,
        title: "Code".into(),
        blocks: vec![md2any::ir::Block::CodeBlock {
            lang: Some("rust".into()),
            title: None,
            lines: vec!["let a = 1;".into(), "let b = 2;".into()],
            line_numbers: true,
            start_line: 42,
            columns: None,
            include_error: None,
        }],
        notes: None,
        bg_image: None,
        layout_hint: None,
    }];
    let bytes = md2any::html::write(
        &slides,
        &theme,
        &layout,
        "Code",
        "tests",
        &assets(),
        None,
        None,
    )
    .unwrap();
    let html = String::from_utf8(bytes).unwrap();

    assert!(html.contains("<span class=\"line-no\">42</span>"), "{html}");
    assert!(html.contains("<span class=\"line-no\">43</span>"), "{html}");
}

#[test]
fn clean_title_underline_is_slide_progress() {
    let md = "# Progress\n## One\nText.\n## Two\nText.\n## Three\nText.\n";
    let (front, body) = md2any::front_matter::extract(md);
    let theme = md2any::theme::Theme::resolve("light", "16:9", None).unwrap();
    let layout = md2any::layout::Layout::resolve("clean").unwrap();
    let slides = md2any::parser::parse(&body, &front, "progress");
    let slides = md2any::paginate::paginate_for_layout(slides, &theme, &layout);
    assert_eq!(slides.len(), 4);

    let html = md2any::html::write(
        &slides,
        &theme,
        &layout,
        "Progress",
        "tests",
        &assets(),
        None,
        None,
    )
    .unwrap();
    let html = String::from_utf8(html).unwrap();
    assert!(html.contains("--slide-progress:50.000%"), "{html}");
    assert!(html.contains("--slide-progress:75.000%"), "{html}");
    assert!(html.contains(".title-underline::after"), "{html}");

    let pdf = md2any::pdf::write(
        &slides,
        &theme,
        &layout,
        "Progress",
        "tests",
        &assets(),
        None,
        None,
        None,
        0.4,
        None,
        false,
        md2any::pdf::NotesPageSize::Slide,
        md2any::pdf::NotesLayout::Auto,
        None,
    )
    .unwrap();
    let accent_rects = pdf_rects_after_color(&pdf, "0.055 0.647 0.914 rg");
    assert!(
        accent_rects
            .iter()
            .any(|(w, h)| *w > 100.0 && (3.0..5.0).contains(h)),
        "expected a wide slide-progress accent rect, got {accent_rects:?}"
    );
}

// ---------------------------------------------------------------------------
// SVG / PNG
// ---------------------------------------------------------------------------

#[test]
fn svg_image_sequence_contains_one_file_per_slide() {
    let md = "---\ntitle: Images\n---\n# Deck\n## Code\n```rust\nlet x = 1;\nlet y = 2;\nlet z = x + y;\nlet q = z + 1;\nlet r = q + 1;\nlet s = r + 1;\n```\n";
    let (front, body) = md2any::front_matter::extract(md);
    let theme = md2any::theme::Theme::resolve("light", "16:9", None).unwrap();
    let layout = md2any::layout::Layout::resolve("clean").unwrap();
    let slides = md2any::parser::parse(&body, &front, "Images");
    let slides = md2any::paginate::paginate_for_layout(slides, &theme, &layout);
    let files = md2any::svg::write_files(
        &slides,
        &theme,
        &layout,
        "Images",
        "tests",
        &assets(),
        None,
        None,
        md2any::svg::ImageFormat::Svg,
    )
    .unwrap();

    assert_eq!(files.len(), slides.len());
    assert_eq!(files[0].name, "slide-001.svg");
    let svg = String::from_utf8(files.last().unwrap().bytes.clone()).unwrap();
    assert!(svg.starts_with("<svg "), "{svg}");
    assert!(svg.contains("<title>Code"), "{svg}");
    assert!(svg.contains(">1</text>"), "{svg}");
}

#[test]
fn svg_text_sizes_and_positions_use_points_not_centipoints() {
    let mut md = String::from("---\ntitle: SVG Units\n---\n# Deck\n## Dense text\n");
    for _ in 0..18 {
        md.push_str("This paragraph is long enough to wrap and force several text nodes in SVG output while still staying within the slide viewport.\n\n");
    }
    md.push_str("```rust\n");
    for i in 0..12 {
        md.push_str(&format!("let value_{i} = compute_value({i});\n"));
    }
    md.push_str("```\n\n| Key | Value |\n|-----|-------|\n| alpha | beta |\n");

    let (front, body) = md2any::front_matter::extract(&md);
    let theme = md2any::theme::Theme::resolve("light", "16:9", None).unwrap();
    let layout = md2any::layout::Layout::resolve("clean").unwrap();
    let slides = md2any::parser::parse(&body, &front, "SVG Units");
    let slides = md2any::paginate::paginate_for_layout(slides, &theme, &layout);
    let files = md2any::svg::write_files(
        &slides,
        &theme,
        &layout,
        "SVG Units",
        "tests",
        &assets(),
        None,
        None,
        md2any::svg::ImageFormat::Svg,
    )
    .unwrap();
    let svg = files
        .iter()
        .map(|file| String::from_utf8(file.bytes.clone()).unwrap())
        .collect::<Vec<_>>()
        .join("\n");

    let font_sizes = xml_attr_numbers(&svg, "font-size");
    assert!(!font_sizes.is_empty(), "{svg}");
    assert!(
        font_sizes.iter().all(|size| *size <= 80.0),
        "SVG font sizes should be points, not centipoints: {font_sizes:?}"
    );

    let y_values = xml_attr_numbers(&svg, "y");
    assert!(!y_values.is_empty(), "{svg}");
    assert!(
        y_values.iter().all(|y| *y <= 620.0),
        "SVG text/image y coordinates should stay near the 540px viewport: {y_values:?}"
    );
}

#[test]
fn png_image_sequence_rasterizes_svg_slides() {
    let theme = md2any::theme::Theme::resolve("light", "16:9", None).unwrap();
    let layout = md2any::layout::Layout::resolve("clean").unwrap();
    let slides = vec![md2any::ir::Slide {
        kind: md2any::ir::SlideKind::Content,
        title: "PNG".into(),
        blocks: vec![md2any::ir::Block::Paragraph(vec![md2any::ir::Run::plain(
            "Raster test.",
        )])],
        notes: None,
        bg_image: None,
        layout_hint: None,
    }];
    let files = md2any::svg::write_files(
        &slides,
        &theme,
        &layout,
        "PNG",
        "tests",
        &assets(),
        None,
        None,
        md2any::svg::ImageFormat::Png,
    )
    .unwrap();

    assert_eq!(files.len(), 1);
    assert_eq!(files[0].name, "slide-001.png");
    assert!(files[0].bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
}

#[test]
fn cli_svg_format_writes_slide_directory() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("md2any-svg-{unique}-{}", std::process::id()));
    let input = root.join("talk.md");
    let output_dir = root.join("slides-svg");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(&input, "---\ntitle: SVG\n---\n# Deck\n## One\nText.\n").unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_md2any"))
        .arg(&input)
        .arg("--format")
        .arg("svg")
        .arg("-o")
        .arg(&output_dir)
        .arg("--quiet")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "svg command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let first = output_dir.join("slide-001.svg");
    assert!(first.exists(), "missing {}", first.display());
    let svg = std::fs::read_to_string(first).unwrap();
    assert!(svg.contains("<svg "), "{svg}");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn cli_code_theme_defaults_dark_and_can_match_main_theme() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("md2any-code-theme-{unique}-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let input = root.join("talk.md");
    let front_matter_input = root.join("front.md");
    let default_out = root.join("default.html");
    let match_out = root.join("match.html");
    let front_out = root.join("front.html");
    std::fs::write(
        &input,
        "---\ntitle: Code Theme\ntheme: light\n---\n# Deck\n## Code\n```rust\nfn main() {}\n```\n",
    )
    .unwrap();
    std::fs::write(
        &front_matter_input,
        "---\ntitle: Code Theme\ntheme: light\ncode_theme: light\n---\n# Deck\n## Code\n```rust\nfn main() {}\n```\n",
    )
    .unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_md2any"))
        .arg(&input)
        .arg("--format")
        .arg("html")
        .arg("-o")
        .arg(&default_out)
        .arg("--quiet")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "default code-theme command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let html = std::fs::read_to_string(&default_out).unwrap();
    assert!(html.contains("--bg: #FFFFFF;"), "{html}");
    assert!(html.contains("--table-band-bg: #F8FAFC;"), "{html}");
    assert!(html.contains("--code-bg: #111A2E;"), "{html}");
    assert!(html.contains("--code-text: #E2E8F0;"), "{html}");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_md2any"))
        .arg(&input)
        .arg("--format")
        .arg("html")
        .arg("-o")
        .arg(&match_out)
        .arg("--code-theme")
        .arg("match")
        .arg("--quiet")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "match code-theme command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let html = std::fs::read_to_string(&match_out).unwrap();
    assert!(html.contains("--bg: #FFFFFF;"), "{html}");
    assert!(html.contains("--code-bg: #F1F5F9;"), "{html}");
    assert!(html.contains("--code-text: #1E293B;"), "{html}");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_md2any"))
        .arg(&front_matter_input)
        .arg("--format")
        .arg("html")
        .arg("-o")
        .arg(&front_out)
        .arg("--quiet")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "front-matter code-theme command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let html = std::fs::read_to_string(&front_out).unwrap();
    assert!(html.contains("--bg: #FFFFFF;"), "{html}");
    assert!(html.contains("--code-bg: #F1F5F9;"), "{html}");
    assert!(html.contains("--code-text: #1E293B;"), "{html}");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn table_band_theme_is_independent_of_code_theme() {
    let mut theme = md2any::theme::Theme::resolve("light", "16:9", None).unwrap();
    assert_eq!(theme.bg, "FFFFFF");
    assert_eq!(theme.code_bg, "111A2E");
    assert_eq!(theme.table_band_bg(), "F8FAFC");

    theme.apply_code_theme(md2any::theme::CodeTheme::Light);
    assert_eq!(theme.code_bg, "F1F5F9");
    assert_eq!(theme.table_band_bg(), "F8FAFC");
}

#[test]
fn cli_help_mentions_serve_format_modes() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_md2any"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "help command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--serve-format"), "{stdout}");
    assert!(stdout.contains("pdf | html | svg | png"), "{stdout}");
}

#[test]
fn cli_help_mentions_doc_style_modes() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_md2any"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "help command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--doc-style"), "{stdout}");
    assert!(
        stdout.contains("plain | report | handout | speaker-notes"),
        "{stdout}"
    );
}

#[test]
fn cli_help_mentions_table_fit_modes() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_md2any"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "help command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--table-fit"), "{stdout}");
    assert!(
        stdout.contains("auto | split | transpose | off"),
        "{stdout}"
    );
}

#[test]
fn cli_help_mentions_code_theme_modes() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_md2any"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "help command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--code-theme"), "{stdout}");
    assert!(stdout.contains("dark | light | match"), "{stdout}");
}

#[test]
fn cli_help_mentions_code_columns_modes() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_md2any"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "help command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--code-columns"), "{stdout}");
    assert!(stdout.contains("single | auto | two-up"), "{stdout}");
}

#[test]
fn cli_help_mentions_math_svg_mode() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_md2any"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "help command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--math"), "{stdout}");
    assert!(stdout.contains("--math-scale"), "{stdout}");
    assert!(stdout.contains("--math-block-align"), "{stdout}");
    assert!(stdout.contains("--math-max-height"), "{stdout}");
    assert!(stdout.contains("unicode | source | svg"), "{stdout}");
}

#[test]
fn deck_doctor_reports_accessibility_and_structure_warnings() {
    use md2any::ir::{Block, ListItem, Run, Slide, SlideKind};

    let mut theme = md2any::theme::Theme::resolve("light", "16:9", None).unwrap();
    theme.bg = "FFFFFF".into();
    theme.body_color = "FFFFFF".into();
    let many_items = (0..34)
        .map(|i| ListItem {
            runs: vec![Run::plain(format!("item {i}"))],
            level: 0,
            ordered: false,
        })
        .collect::<Vec<_>>();
    let slides = vec![
        Slide {
            kind: SlideKind::Content,
            title: "Repeat".into(),
            blocks: vec![
                Block::Image {
                    src: "missing.png".into(),
                    alt: String::new(),
                    width_pct: None,
                },
                Block::Columns {
                    left: Vec::new(),
                    right: vec![Block::Paragraph(vec![Run::plain("right only")])],
                },
                Block::Table {
                    headers: Vec::new(),
                    rows: vec![vec![vec![Run::plain("cell")]; 4]; 24],
                    aligns: Vec::new(),
                },
                Block::List(many_items),
            ],
            notes: Some("Only one slide has notes".into()),
            bg_image: None,
            layout_hint: None,
        },
        Slide {
            kind: SlideKind::Content,
            title: "Repeat".into(),
            blocks: Vec::new(),
            notes: None,
            bg_image: None,
            layout_hint: None,
        },
        Slide {
            kind: SlideKind::Content,
            title: "Other".into(),
            blocks: Vec::new(),
            notes: None,
            bg_image: None,
            layout_hint: None,
        },
    ];

    let warnings = md2any::lint::check(&slides, &theme);
    let kinds = warnings.iter().map(|w| w.kind).collect::<Vec<_>>();
    for expected in [
        "low-body-contrast",
        "incomplete-speaker-notes",
        "duplicate-title",
        "dense-slide",
        "missing-alt-text",
        "empty-column",
        "table-without-header",
        "large-table",
    ] {
        assert!(kinds.contains(&expected), "missing {expected}: {kinds:?}");
    }
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
                assert!(
                    xml.contains(r#"<a:srgbClr val="F8FAFC"/>"#),
                    "table band should use neutral light fill, not code background: {xml}"
                );
                found = true;
                break;
            }
        }
    }
    assert!(found, "expected at least one <a:tbl> in slides");
}

#[test]
fn pptx_wraps_long_table_cells_with_weighted_columns() {
    let bytes = render_markdown("pptx", long_capability_table_md());
    let mut table_xml = String::new();
    let cursor = std::io::Cursor::new(&bytes);
    let mut archive = zip::ZipArchive::new(cursor).unwrap();
    for i in 0..archive.len() {
        let name = archive.by_index(i).unwrap().name().to_string();
        if name.starts_with("ppt/slides/slide") && name.ends_with(".xml") {
            let xml = zip_read(&bytes, &name);
            if let Some(start) = xml.find("<a:tbl>") {
                let tail = &xml[start..];
                let end = tail.find("</a:tbl>").unwrap() + "</a:tbl>".len();
                table_xml = tail[..end].to_string();
                break;
            }
        }
    }
    assert!(!table_xml.is_empty(), "expected a table in generated PPTX");
    let grid = table_xml
        .split("<a:tblGrid>")
        .nth(1)
        .and_then(|s| s.split("</a:tblGrid>").next())
        .unwrap();
    let widths = xml_attr_numbers(grid, "w");
    assert_eq!(widths.len(), 2, "expected two table columns: {grid}");
    assert!(
        widths[1] > widths[0] * 2.0,
        "narrative column should be much wider than label column: {widths:?}"
    );
    let row_heights = xml_attr_numbers(&table_xml, "h");
    assert!(
        row_heights.iter().any(|h| *h > 500000.0),
        "wrapped rows should be taller than a fixed single-line row: {row_heights:?}"
    );
    assert!(
        table_xml.matches("<a:p>").count() > 10,
        "wrapped cells should emit multiple paragraphs: {table_xml}"
    );
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
    assert!(
        content.contains(r#"svg:height="0.0500cm""#),
        "missing themed footer rule"
    );
}

#[test]
fn odp_wraps_long_table_cells_with_weighted_columns() {
    let bytes = render_markdown("odp", long_capability_table_md());
    let content = zip_read(&bytes, "content.xml");
    assert!(
        content.contains("Section</text:span>")
            && content.contains("presenter</text:span>")
            && content.contains("</text:p><text:p"),
        "expected hard-wrapped table cell text in ODP content: {content}"
    );
    let widths = xml_attr_numbers(&content, "svg:width");
    assert!(
        widths.windows(2).any(|pair| pair[1] > pair[0] * 2.0),
        "expected a weighted two-column table in ODP widths: {widths:?}"
    );
    let heights = xml_attr_numbers(&content, "svg:height");
    assert!(
        heights.iter().any(|h| *h > 1.4 && *h < 3.0),
        "expected wrapped table rows taller than fixed single-line rows: {heights:?}"
    );
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

#[test]
fn docx_report_profile_adds_contents_and_page_chrome() {
    let bytes = render("docx");
    zip_contains(
        &bytes,
        &[
            "word/header1.xml",
            "word/footer1.xml",
            "word/_rels/document.xml.rels",
        ],
    )
    .unwrap();
    let doc = zip_read(&bytes, "word/document.xml");
    let rels = zip_read(&bytes, "word/_rels/document.xml.rels");
    let styles = zip_read(&bytes, "word/styles.xml");
    assert!(doc.contains("Contents"), "missing static contents section");
    assert!(doc.contains("TocEntry"), "missing contents entries");
    assert!(doc.contains("headerReference"), "missing header reference");
    assert!(doc.contains("footerReference"), "missing footer reference");
    assert!(rels.contains("header1.xml"), "missing header relationship");
    assert!(rels.contains("footer1.xml"), "missing footer relationship");
    assert!(
        styles.contains("DocMeta"),
        "missing document metadata style"
    );
    assert!(
        styles.contains(r#"<w:bottom w:val="single" w:sz="12""#),
        "missing heading accent underline"
    );
    assert!(
        styles.contains(r#"<w:left w:val="single" w:sz="16""#),
        "missing code-block accent edge"
    );
    assert!(
        styles.contains(r#"<w:shd w:val="clear" w:color="auto" w:fill="E0F2FE""#),
        "missing Heading2 accent-soft shading"
    );
}

#[test]
fn docx_speaker_notes_profile_appends_notes() {
    let md =
        "---\ntitle: Notes doc\n---\n# Deck\n## First\nVisible.\n<!-- notes: Say this aloud. -->\n";
    let (front, body) = md2any::front_matter::extract(md);
    let theme = md2any::theme::Theme::resolve("light", "16:9", None).unwrap();
    let slides = md2any::parser::parse(&body, &front, "notes");
    let slides = md2any::paginate::paginate(slides, &theme);
    let options =
        md2any::document::DocumentOptions::new(md2any::document::DocumentStyle::SpeakerNotes);
    let bytes = md2any::docx::write_with_options(
        &slides,
        &theme,
        "Notes doc",
        "tests",
        &assets(),
        None,
        None,
        &options,
    )
    .unwrap();
    let doc = zip_read(&bytes, "word/document.xml");
    assert!(doc.contains("Speaker notes"), "{doc}");
    assert!(doc.contains("Say this aloud."), "{doc}");
    assert!(doc.contains("NotesBody"), "{doc}");
    assert!(doc.contains("SlideLabel"), "{doc}");
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

#[test]
fn odt_report_profile_adds_contents_and_page_chrome() {
    let bytes = render("odt");
    let content = zip_read(&bytes, "content.xml");
    let styles = zip_read(&bytes, "styles.xml");
    assert!(content.contains("Contents"), "missing contents section");
    assert!(content.contains("TocEntry"), "missing contents entries");
    assert!(styles.contains("<style:header>"), "missing ODT header");
    assert!(styles.contains("<style:footer>"), "missing ODT footer");
    assert!(
        styles.contains("DocMeta"),
        "missing document metadata style"
    );
}

#[test]
fn odt_speaker_notes_profile_appends_notes() {
    let md =
        "---\ntitle: Notes doc\n---\n# Deck\n## First\nVisible.\n<!-- notes: Say this aloud. -->\n";
    let (front, body) = md2any::front_matter::extract(md);
    let theme = md2any::theme::Theme::resolve("light", "16:9", None).unwrap();
    let slides = md2any::parser::parse(&body, &front, "notes");
    let slides = md2any::paginate::paginate(slides, &theme);
    let options =
        md2any::document::DocumentOptions::new(md2any::document::DocumentStyle::SpeakerNotes);
    let bytes = md2any::odt::write_with_options(
        &slides,
        &theme,
        "Notes doc",
        "tests",
        &assets(),
        None,
        None,
        &options,
    )
    .unwrap();
    let content = zip_read(&bytes, "content.xml");
    assert!(content.contains("Speaker notes"), "{content}");
    assert!(content.contains("Say this aloud."), "{content}");
    assert!(content.contains("NotesBody"), "{content}");
    assert!(content.contains("SlideLabel"), "{content}");
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
