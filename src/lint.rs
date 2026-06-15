//! Optional `--check` mode. Walks the post-paginated slide tree and reports
//! issues that won't cause a build failure but would hurt the final deck:
//! over-long titles, narrow tables in portrait, code lines that won't wrap
//! cleanly, etc. Writes warnings to stderr and returns the count.

use crate::ir::*;
use crate::theme::Theme;
use std::collections::HashMap;

pub struct Warning {
    pub slide: usize,
    pub kind: &'static str,
    pub detail: String,
}

pub fn check(slides: &[Slide], theme: &Theme) -> Vec<Warning> {
    let mut out = Vec::new();
    check_theme_contrast(theme, &mut out);
    check_notes_coverage(slides, &mut out);
    let mut titles: HashMap<String, usize> = HashMap::new();
    for (i, slide) in slides.iter().enumerate() {
        let num = i + 1;
        check_title(num, slide, &mut out);
        check_duplicate_title(num, slide, &mut titles, &mut out);
        check_slide_density(num, slide, &mut out);
        check_blocks(num, &slide.blocks, theme, &mut out);
    }
    out
}

fn check_title(num: usize, slide: &Slide, out: &mut Vec<Warning>) {
    let words = slide.title.split_whitespace().count();
    if matches!(slide.kind, SlideKind::Content) && words > 8 {
        out.push(Warning {
            slide: num,
            kind: "long-title",
            detail: format!("title is {} words; aim for ≤ 7 for visual weight", words),
        });
    }
    if slide.title.chars().count() > 70 {
        out.push(Warning {
            slide: num,
            kind: "wide-title",
            detail: format!(
                "title is {} chars; will wrap to two lines on most layouts",
                slide.title.chars().count()
            ),
        });
    }
}

fn check_duplicate_title(
    num: usize,
    slide: &Slide,
    seen: &mut HashMap<String, usize>,
    out: &mut Vec<Warning>,
) {
    if slide.title.ends_with(" (cont.)") {
        return;
    }
    let normalized = trim_cont_suffix(&slide.title)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    if normalized.is_empty() {
        return;
    }
    if let Some(first) = seen.insert(normalized, num) {
        out.push(Warning {
            slide: num,
            kind: "duplicate-title",
            detail: format!(
                "title repeats slide {}; duplicate headings make review and exports harder to scan",
                first
            ),
        });
    }
}

fn check_slide_density(num: usize, slide: &Slide, out: &mut Vec<Warning>) {
    if !matches!(slide.kind, SlideKind::Content) {
        return;
    }
    let weight = slide.blocks.iter().map(block_weight).sum::<f32>();
    if weight > 40.0 {
        out.push(Warning {
            slide: num,
            kind: "dense-slide",
            detail: format!(
                "content density score {:.1}; consider splitting, summarising, or using --break-fill below 100",
                weight
            ),
        });
    }
}

fn check_notes_coverage(slides: &[Slide], out: &mut Vec<Warning>) {
    let note_count = slides.iter().filter(|s| s.notes.is_some()).count();
    if note_count == 0 {
        return;
    }
    let talk_slides = slides
        .iter()
        .filter(|s| !matches!(s.kind, SlideKind::Title { .. }))
        .count();
    let missing = talk_slides.saturating_sub(note_count);
    if missing > 0 && note_count * 2 < talk_slides.max(1) {
        out.push(Warning {
            slide: 0,
            kind: "incomplete-speaker-notes",
            detail: format!(
                "{} of {} non-title slides have notes; speaker exports may look uneven",
                note_count, talk_slides
            ),
        });
    }
}

fn check_theme_contrast(theme: &Theme, out: &mut Vec<Warning>) {
    for (kind, fg, bg, target) in [
        ("low-body-contrast", &theme.body_color, &theme.bg, 4.5),
        ("low-title-contrast", &theme.title_color, &theme.bg, 3.0),
        (
            "low-section-contrast",
            &theme.section_text,
            &theme.section_bg,
            3.0,
        ),
    ] {
        let Some(ratio) = contrast_ratio(fg, bg) else {
            continue;
        };
        if ratio < target {
            out.push(Warning {
                slide: 0,
                kind,
                detail: format!(
                    "{} on {} contrast is {:.2}:1; aim for at least {:.1}:1",
                    fg, bg, ratio, target
                ),
            });
        }
    }
}

fn check_blocks(num: usize, blocks: &[Block], theme: &Theme, out: &mut Vec<Warning>) {
    for b in blocks {
        match b {
            Block::Table { headers, rows, .. } => {
                let cols = headers
                    .len()
                    .max(rows.iter().map(|r| r.len()).max().unwrap_or(0));
                if theme.portrait && cols > 6 {
                    out.push(Warning {
                        slide: num,
                        kind: "wide-table-in-portrait",
                        detail: format!(
                            "{} columns in portrait — font will scale down; consider rotating to landscape or splitting",
                            cols
                        ),
                    });
                } else if !theme.portrait && cols > 10 {
                    out.push(Warning {
                        slide: num,
                        kind: "very-wide-table",
                        detail: format!("{} columns — likely too narrow per column to read", cols),
                    });
                }
                if rows.len() > 14 {
                    out.push(Warning {
                        slide: num,
                        kind: "long-table",
                        detail: format!("{} rows on one slide — split into two?", rows.len()),
                    });
                }
                if headers.is_empty() {
                    out.push(Warning {
                        slide: num,
                        kind: "table-without-header",
                        detail: "table has no header row; accessibility and document exports lose structure".into(),
                    });
                }
                if cols * rows.len().max(1) > 90 {
                    out.push(Warning {
                        slide: num,
                        kind: "large-table",
                        detail: format!(
                            "{} cells; consider splitting, transposing, or summarising top rows",
                            cols * rows.len().max(1)
                        ),
                    });
                }
            }
            Block::CodeBlock {
                lines,
                include_error,
                ..
            } => {
                if let Some(error) = include_error {
                    out.push(Warning {
                        slide: num,
                        kind: "code-include-failed",
                        detail: format!("source include failed: {error}"),
                    });
                }
                let max_chars = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
                let threshold = if theme.portrait { 70 } else { 100 };
                if max_chars > threshold {
                    out.push(Warning {
                        slide: num,
                        kind: "long-code-line",
                        detail: format!(
                            "longest code line is {} chars (will auto-shrink and/or wrap); consider line-wrapping the source",
                            max_chars
                        ),
                    });
                }
                if lines.len() > 16 {
                    out.push(Warning {
                        slide: num,
                        kind: "long-code-block",
                        detail: format!(
                            "{} lines of code on one slide — will paginate or be hard to read",
                            lines.len()
                        ),
                    });
                }
            }
            Block::List(items) => {
                if items.len() > crate::theme::LONG_LIST_THRESHOLD && theme.portrait {
                    out.push(Warning {
                        slide: num,
                        kind: "long-list-in-portrait",
                        detail: format!(
                            "{} items — auto-2-column wrap is disabled in portrait; consider splitting",
                            items.len()
                        ),
                    });
                }
            }
            Block::Columns { left, right } => {
                if left.is_empty() || right.is_empty() {
                    out.push(Warning {
                        slide: num,
                        kind: "empty-column",
                        detail:
                            "column layout has an empty side; reading order may surprise reviewers"
                                .into(),
                    });
                }
                check_blocks(num, left, theme, out);
                check_blocks(num, right, theme, out);
            }
            Block::Paragraph(runs) => {
                let chars = runs.iter().map(|r| r.text.chars().count()).sum::<usize>();
                if chars > 600 {
                    out.push(Warning {
                        slide: num,
                        kind: "wall-of-text",
                        detail: format!(
                            "paragraph is {} chars — break into bullets or split slides",
                            chars
                        ),
                    });
                }
            }
            Block::Image { src, alt, .. } => {
                if alt.trim().is_empty() {
                    out.push(Warning {
                        slide: num,
                        kind: "missing-alt-text",
                        detail: format!(
                            "image {} has no alt text; add ![description](...) for accessibility and document captions",
                            src
                        ),
                    });
                }
            }
            _ => {}
        }
    }
}

pub fn report(warnings: &[Warning]) {
    if warnings.is_empty() {
        eprintln!("md2any --check: no issues found");
        return;
    }
    eprintln!("md2any --check: {} warning(s)", warnings.len());
    for w in warnings {
        if w.slide == 0 {
            eprintln!("  deck: [{}] {}", w.kind, w.detail);
        } else {
            eprintln!("  slide {}: [{}] {}", w.slide, w.kind, w.detail);
        }
    }
}

fn block_weight(block: &Block) -> f32 {
    match block {
        Block::Paragraph(runs) => {
            let chars = runs.iter().map(|r| r.text.chars().count()).sum::<usize>();
            2.0 + chars as f32 / 90.0
        }
        Block::Heading { runs, .. } => {
            2.0 + runs.iter().map(|r| r.text.chars().count()).sum::<usize>() as f32 / 80.0
        }
        Block::List(items) => items.len() as f32 * 1.35,
        Block::CodeBlock { lines, .. } => lines.len() as f32 * 1.15 + 2.0,
        Block::Quote(paras) | Block::Callout { body: paras, .. } => {
            2.0 + paras
                .iter()
                .flatten()
                .map(|r| r.text.chars().count())
                .sum::<usize>() as f32
                / 100.0
        }
        Block::Table { headers, rows, .. } => {
            let cols = headers
                .len()
                .max(rows.iter().map(|r| r.len()).max().unwrap_or(0));
            4.0 + (rows.len() + usize::from(!headers.is_empty())) as f32 * cols as f32 * 0.55
        }
        Block::Columns { left, right } => {
            left.iter().map(block_weight).sum::<f32>() + right.iter().map(block_weight).sum::<f32>()
        }
        Block::Image { .. } => 8.0,
        Block::Cards { cards, .. } => cards_as_blocks(cards).iter().map(block_weight).sum(),
        Block::Footnotes(items) => 1.0 + items.len() as f32,
        Block::ColumnBreak => 0.0,
    }
}

fn contrast_ratio(fg: &str, bg: &str) -> Option<f32> {
    let fg = relative_luminance(parse_hex_rgb(fg)?);
    let bg = relative_luminance(parse_hex_rgb(bg)?);
    let light = fg.max(bg);
    let dark = fg.min(bg);
    Some((light + 0.05) / (dark + 0.05))
}

fn parse_hex_rgb(value: &str) -> Option<(f32, f32, f32)> {
    let hex = value.trim().trim_start_matches('#');
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()? as f32 / 255.0;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()? as f32 / 255.0;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()? as f32 / 255.0;
    Some((r, g, b))
}

fn relative_luminance((r, g, b): (f32, f32, f32)) -> f32 {
    fn channel(v: f32) -> f32 {
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

fn trim_cont_suffix(s: &str) -> &str {
    s.strip_suffix(" (cont.)").unwrap_or(s)
}
