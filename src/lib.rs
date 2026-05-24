//! md2any — Markdown → PowerPoint / OpenDocument / PDF / Word / Writer.
//!
//! ## Pipeline overview
//!
//! ```text
//!  markdown source
//!        ↓
//!  front_matter::extract       (split YAML header from body)
//!        ↓
//!  parser::parse               (pulldown-cmark → IR slides)
//!        ↓
//!  diagram::pre_render         (shell out for dot/mermaid/plantuml)
//!        ↓
//!  paginate::paginate          (split overlong slides into "(cont.)")
//!        ↓
//!  toc::inject (optional)      (auto Contents slide)
//!        ↓
//!  one of: pptx | odp | pdf | docx | odt ::write
//!        ↓
//!  bytes
//! ```
//!
//! Every step operates on plain in-memory data — there's no disk I/O after
//! the initial markdown read until the final byte buffer is written out.
//! That keeps the whole conversion in the millisecond range and makes
//! `--watch` / `--serve` trivial to implement.
//!
//! ## Crate layout
//!
//! - [`front_matter`], [`parser`], [`paginate`], [`toc`] — input pipeline.
//! - [`ir`] — slide tree shared by every renderer.
//! - [`theme`], [`layout`], [`syntax`] — visual configuration.
//! - [`pptx`], [`odp`], [`pdf`], [`docx`], [`odt`] — the five writers.
//! - [`diagram`], [`math`] — preprocessing helpers.
//! - [`lint`] — `--check` mode.
//! - [`serve`] — `--serve` HTTP preview.
//! - [`image`] — PNG/JPEG sniffer + dimension reader.
//!
//! ## Library use
//!
//! The CLI binary in `main.rs` is the canonical consumer, but the same
//! modules are reachable from integration tests and external code. A
//! minimal embed looks like:
//!
//! ```no_run
//! let md = std::fs::read_to_string("talk.md")?;
//! let (front, body) = md2any::front_matter::extract(&md);
//! let theme = md2any::theme::Theme::resolve("light", "16:9", None)?;
//! let layout = md2any::layout::Layout::resolve("clean")?;
//! let slides = md2any::parser::parse(&body, &front, "talk");
//! let slides = md2any::paginate::paginate(slides, &theme);
//! let bytes = md2any::pdf::write(
//!     &slides, &theme, &layout, "talk", "Author",
//!     std::path::Path::new("."), None, None, None, 0.4, None, false, None,
//! )?;
//! std::fs::write("talk.pdf", bytes)?;
//! # Ok::<(), anyhow::Error>(())
//! ```

pub mod diagram;
pub mod docx;
pub mod font;
pub mod front_matter;
pub mod image;
pub mod ir;
pub mod layout;
pub mod lint;
pub mod math;
pub mod odp;
pub mod odt;
pub mod paginate;
pub mod parser;
pub mod pdf;
pub mod pptx;
pub mod serve;
pub mod syntax;
pub mod theme;
pub mod toc;

/// Render the parsed/paginated slide tree as a stable, human-readable text
/// snapshot. Used by golden-file tests to assert that parser changes don't
/// silently alter the deck structure.
pub fn slides_snapshot(slides: &[ir::Slide]) -> String {
    use ir::*;
    let mut out = String::new();
    for (i, s) in slides.iter().enumerate() {
        let kind = match &s.kind {
            SlideKind::Title {
                subtitle,
                author,
                date,
            } => format!(
                "Title(subtitle={:?}, author={:?}, date={:?})",
                subtitle, author, date
            ),
            SlideKind::Section => "Section".into(),
            SlideKind::Content => "Content".into(),
        };
        out.push_str(&format!("=== slide {} [{}] ===\n", i + 1, kind));
        out.push_str(&format!("title: {}\n", s.title));
        if let Some(bg) = &s.bg_image {
            out.push_str(&format!("bg: {}\n", bg));
        }
        if let Some(n) = &s.notes {
            out.push_str(&format!("notes: {}\n", n.replace('\n', " ⏎ ")));
        }
        for b in &s.blocks {
            block_snapshot(b, 0, &mut out);
        }
        out.push('\n');
    }
    out
}

fn block_snapshot(b: &ir::Block, depth: usize, out: &mut String) {
    use ir::*;
    let pad = "  ".repeat(depth);
    match b {
        Block::Paragraph(runs) => {
            out.push_str(&format!("{}P: {}\n", pad, runs_repr(runs)));
        }
        Block::Heading { level, runs } => {
            out.push_str(&format!("{}H{}: {}\n", pad, level, runs_repr(runs)));
        }
        Block::List(items) => {
            out.push_str(&format!("{}List:\n", pad));
            for item in items {
                let mark = if item.ordered { "1." } else { "-" };
                out.push_str(&format!(
                    "{}  {}{} {}\n",
                    pad,
                    "  ".repeat(item.level as usize),
                    mark,
                    runs_repr(&item.runs)
                ));
            }
        }
        Block::CodeBlock {
            lang,
            title,
            lines,
            line_numbers,
        } => {
            out.push_str(&format!(
                "{}Code(lang={:?}, title={:?}, lines={}, nums={}):\n",
                pad,
                lang,
                title,
                lines.len(),
                line_numbers
            ));
            for line in lines {
                out.push_str(&format!("{}  | {}\n", pad, line));
            }
        }
        Block::Quote(paras) => {
            out.push_str(&format!("{}Quote:\n", pad));
            for para in paras {
                out.push_str(&format!("{}  > {}\n", pad, runs_repr(para)));
            }
        }
        Block::Table { headers, rows } => {
            out.push_str(&format!(
                "{}Table(cols={}, rows={}):\n",
                pad,
                headers.len(),
                rows.len()
            ));
            out.push_str(&format!(
                "{}  H: {}\n",
                pad,
                headers
                    .iter()
                    .map(|c| runs_repr(c))
                    .collect::<Vec<_>>()
                    .join(" | ")
            ));
            for row in rows {
                out.push_str(&format!(
                    "{}  R: {}\n",
                    pad,
                    row.iter()
                        .map(|c| runs_repr(c))
                        .collect::<Vec<_>>()
                        .join(" | ")
                ));
            }
        }
        Block::ColumnBreak => out.push_str(&format!("{}---ColumnBreak---\n", pad)),
        Block::Columns { left, right } => {
            out.push_str(&format!("{}Columns:\n", pad));
            out.push_str(&format!("{}  Left:\n", pad));
            for b in left {
                block_snapshot(b, depth + 2, out);
            }
            out.push_str(&format!("{}  Right:\n", pad));
            for b in right {
                block_snapshot(b, depth + 2, out);
            }
        }
        Block::Image {
            src,
            alt,
            width_pct: _,
        } => {
            out.push_str(&format!("{}Image({}, alt={:?})\n", pad, src, alt));
        }
        Block::Footnotes(items) => {
            out.push_str(&format!("{}Footnotes:\n", pad));
            for item in items {
                out.push_str(&format!("{}  • {}\n", pad, runs_repr(&item.runs)));
            }
        }
    }
}

fn runs_repr(runs: &[ir::Run]) -> String {
    let mut s = String::new();
    for r in runs {
        let mut markers = String::new();
        if r.bold {
            markers.push('B');
        }
        if r.italic {
            markers.push('I');
        }
        if r.code {
            markers.push('C');
        }
        if r.strike {
            markers.push('S');
        }
        if r.link.is_some() {
            markers.push('L');
        }
        if !markers.is_empty() {
            s.push_str(&format!("[{}]{}", markers, r.text));
        } else {
            s.push_str(&r.text);
        }
    }
    s
}
