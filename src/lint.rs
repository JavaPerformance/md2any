//! Optional `--check` mode. Walks the post-paginated slide tree and reports
//! issues that won't cause a build failure but would hurt the final deck:
//! over-long titles, narrow tables in portrait, code lines that won't wrap
//! cleanly, etc. Writes warnings to stderr and returns the count.

use crate::ir::*;
use crate::theme::Theme;

pub struct Warning {
    pub slide: usize,
    pub kind: &'static str,
    pub detail: String,
}

pub fn check(slides: &[Slide], theme: &Theme) -> Vec<Warning> {
    let mut out = Vec::new();
    for (i, slide) in slides.iter().enumerate() {
        let num = i + 1;
        check_title(num, slide, &mut out);
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

fn check_blocks(num: usize, blocks: &[Block], theme: &Theme, out: &mut Vec<Warning>) {
    for b in blocks {
        match b {
            Block::Table { headers, rows } => {
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
            }
            Block::CodeBlock { lines, .. } => {
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
        eprintln!("  slide {}: [{}] {}", w.slide, w.kind, w.detail);
    }
}
