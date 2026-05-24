//! Slide pagination — splits a parsed deck into rendered slides.
//!
//! md2any parses the markdown into one logical "slide" per H1/H2 with all
//! following blocks attached. This module's job is to take that intent and
//! decide where each slide actually ends, given how much vertical room the
//! chosen aspect ratio offers. Output is a fresh `Vec<Slide>` ready for
//! the renderers.
//!
//! The model is weight-based, not pixel-based. Each block declares a
//! synthetic "weight" (roughly: how many body-line equivalents it occupies
//! once rendered) and the paginator stops adding blocks to a slide once
//! the running total would exceed the slide budget. Code blocks and large
//! tables additionally know how to chunk themselves across pages, with
//! `(cont.)` suffixes applied to the carry-on slide titles.

use crate::ir::*;
use crate::theme::Theme;

/// Body-line budget for one slide of the given theme.
///
/// Reference point: 14 units fits a 16:9 widescreen slide
/// (slide_h = 6_858_000 EMU = 7.5 in). Title bar + footer + slide padding
/// consume a roughly *constant* ~2 in regardless of aspect, so the budget
/// is computed against (slide_h - chrome), not the full height. Without
/// subtracting chrome, tall aspects (A4 / 9:16) get an inflated budget
/// because the chrome scales linearly with them when it shouldn't.
fn budget_for_theme(theme: &Theme) -> f32 {
    let chrome_emu = 1_828_800.0; // ≈ 2 in title + footer + padding
    let content_h = (theme.slide_h as f32 - chrome_emu).max(914_400.0);
    let per_unit = (6_858_000.0 - chrome_emu) / 14.0;
    content_h / per_unit
}

/// Width-aware multiplier for the `chars / N` line-count estimates in
/// [`block_weight`]. The hardcoded divisors (85, 75, 70, 110) were tuned
/// against 16:9 (slide_w = 12,192,000 EMU). Portrait aspects are ~60% as
/// wide, so the same paragraph wraps to proportionally more lines; this
/// scale factor stretches the divisor to match.
fn width_scale(theme: &Theme) -> f32 {
    let ref_w = 12_192_000.0;
    (theme.slide_w as f32 / ref_w).clamp(0.5, 1.2)
}

/// Code-block vertical weight per line, relative to one body line.
/// Code renders at ~14pt monospace with ~1.2 leading vs ~17pt body with
/// ~1.4 leading, so the rendered ratio is ~0.7. Anything materially lower
/// makes long code blocks overflow the bottom of 16:9 slides; anything
/// higher splits 30-line reference listings into near-empty pages on A4.
const CODE_LINE_WEIGHT: f32 = 0.7;

pub fn paginate(slides: Vec<Slide>, theme: &Theme) -> Vec<Slide> {
    let budget = budget_for_theme(theme);
    let wscale = width_scale(theme);
    let mut out = Vec::new();
    for mut slide in slides {
        slide.blocks = coalesce_columns(std::mem::take(&mut slide.blocks));

        if matches!(slide.kind, SlideKind::Section) {
            let blocks = std::mem::take(&mut slide.blocks);
            let title = slide.title.clone();
            let notes = slide.notes.clone();
            let bg = slide.bg_image.clone();
            out.push(slide);
            if !blocks.is_empty() {
                let follow = Slide {
                    kind: SlideKind::Content,
                    title,
                    blocks,
                    notes,
                    bg_image: bg,
                    layout_hint: None,
                };
                for s in paginate(vec![follow], theme) {
                    out.push(s);
                }
            }
            continue;
        }
        if !matches!(slide.kind, SlideKind::Content) {
            out.push(slide);
            continue;
        }
        let total: f32 = slide.blocks.iter().map(|b| block_weight(b, wscale)).sum();
        if total <= budget {
            out.push(slide);
            continue;
        }

        let mut current = Slide {
            kind: SlideKind::Content,
            title: slide.title.clone(),
            blocks: Vec::new(),
            notes: None,
            bg_image: slide.bg_image.clone(),
            layout_hint: slide.layout_hint.clone(),
        };
        let mut weight = 0.0_f32;
        // The "(cont.)" suffix only applies once a real first page has
        // been emitted — if orphan-carry swallowed the initial attempt,
        // the page that finally ships is still "page 1" of the slide.
        let mut emitted_first = false;

        for block in slide.blocks {
            let w = block_weight(&block, wscale);
            if !current.blocks.is_empty() && weight + w > budget {
                // Widow/orphan control: if the last block on the current
                // page is a short "lead-in" (e.g. "Here are the rules:"),
                // carry it to the next page so it stays with the content
                // it introduces rather than dangling at a page break.
                let orphan_lead = current.blocks.last().map_or(false, is_lead_in);
                let carried_lead = if orphan_lead {
                    current.blocks.pop()
                } else {
                    None
                };

                if !current.blocks.is_empty() {
                    out.push(current);
                    emitted_first = true;
                }
                let cont_title = if emitted_first && !slide.title.ends_with(" (cont.)") {
                    format!("{} (cont.)", slide.title)
                } else {
                    slide.title.clone()
                };
                current = Slide {
                    kind: SlideKind::Content,
                    title: cont_title,
                    blocks: Vec::new(),
                    notes: None,
                    bg_image: slide.bg_image.clone(),
                    layout_hint: slide.layout_hint.clone(),
                };
                weight = 0.0;

                if let Some(lead) = carried_lead {
                    weight += block_weight(&lead, wscale);
                    current.blocks.push(lead);
                }
            }

            if matches!(&block, Block::CodeBlock { lines, .. } if (lines.len() as f32) * CODE_LINE_WEIGHT + 1.5 > budget + 2.0)
            {
                if let Block::CodeBlock {
                    lang,
                    title,
                    lines,
                    line_numbers,
                } = block
                {
                    let chunk_size = ((budget / CODE_LINE_WEIGHT) as usize)
                        .saturating_sub(2)
                        .max(6);
                    for (idx, chunk) in lines.chunks(chunk_size).enumerate() {
                        if !current.blocks.is_empty() && idx > 0 {
                            out.push(current);
                            emitted_first = true;
                            current = Slide {
                                kind: SlideKind::Content,
                                title: if emitted_first && !slide.title.ends_with(" (cont.)") {
                                    format!("{} (cont.)", slide.title)
                                } else {
                                    slide.title.clone()
                                },
                                blocks: Vec::new(),
                                notes: None,
                                bg_image: slide.bg_image.clone(),
                                layout_hint: slide.layout_hint.clone(),
                            };
                            weight = 0.0;
                        }
                        current.blocks.push(Block::CodeBlock {
                            lang: lang.clone(),
                            title: if idx == 0 { title.clone() } else { None },
                            lines: chunk.to_vec(),
                            line_numbers,
                        });
                        weight += chunk.len() as f32 + 1.5;
                    }
                }
                continue;
            }

            // Mirror of the code-block branch for GFM tables. The header
            // row is repeated on each continuation slide so a reader who
            // lands on page N still sees the column titles.
            if matches!(&block, Block::Table { rows, .. } if (rows.len() as f32 + 1.0) * 1.3 > budget + 1.0)
            {
                if let Block::Table { headers, rows } = block {
                    // -1 leaves room for the header row that re-renders
                    // on every chunk.
                    let max_rows = (((budget - 1.0) / 1.3) as usize).saturating_sub(1).max(4);
                    for (idx, chunk) in rows.chunks(max_rows).enumerate() {
                        if !current.blocks.is_empty() && idx > 0 {
                            out.push(current);
                            emitted_first = true;
                            current = Slide {
                                kind: SlideKind::Content,
                                title: if emitted_first && !slide.title.ends_with(" (cont.)") {
                                    format!("{} (cont.)", slide.title)
                                } else {
                                    slide.title.clone()
                                },
                                blocks: Vec::new(),
                                notes: None,
                                bg_image: slide.bg_image.clone(),
                                layout_hint: slide.layout_hint.clone(),
                            };
                            weight = 0.0;
                        }
                        current.blocks.push(Block::Table {
                            headers: headers.clone(),
                            rows: chunk.to_vec(),
                        });
                        weight += (chunk.len() as f32 + 1.0) * 1.3;
                    }
                }
                continue;
            }

            weight += w;
            current.blocks.push(block);
        }

        if !current.blocks.is_empty() {
            out.push(current);
        }
    }
    out
}

fn block_weight(b: &Block, wscale: f32) -> f32 {
    match b {
        Block::Paragraph(runs) => {
            let chars = total_chars(runs) as f32;
            (chars / (85.0 * wscale)).max(1.0) + 0.4
        }
        Block::Heading { .. } => 1.6,
        Block::List(items) => {
            let total: f32 = items
                .iter()
                .map(|i| {
                    let chars = i.runs.iter().map(|r| r.text.chars().count()).sum::<usize>() as f32;
                    let lines = (chars / (75.0 * wscale)).max(1.0);
                    lines + 0.1 * i.level as f32
                })
                .sum();
            // Long lists of short bullets render denser than the per-item
            // calculation suggests. Only halve when EVERY item is short
            // enough to fit on one line — a list with even one wrapping
            // bullet (e.g. a long sentence) needs its real height.
            let all_short = items.iter().all(|i| {
                let chars = i.runs.iter().map(|r| r.text.chars().count()).sum::<usize>() as f32;
                chars <= 75.0 * wscale
            });
            if items.len() > crate::theme::LONG_LIST_THRESHOLD && all_short {
                (total / 2.0).max(items.len() as f32 / 2.0)
            } else {
                total
            }
        }
        Block::CodeBlock { lines, title, .. } => {
            lines.len() as f32 * CODE_LINE_WEIGHT + 1.5 + if title.is_some() { 0.6 } else { 0.0 }
        }
        Block::Quote(paras) => {
            paras
                .iter()
                .map(|runs| (total_chars(runs) as f32 / (70.0 * wscale)).max(1.0))
                .sum::<f32>()
                + 0.6
        }
        Block::Table { rows, .. } => (rows.len() as f32 + 1.0) * 1.3,
        Block::ColumnBreak => 0.0,
        Block::Columns { left, right } => {
            // Two-column layout halves the per-column width, so each side
            // wraps to roughly twice as many lines as it would full-width.
            let col_scale = wscale * 0.5;
            let lw: f32 = left.iter().map(|b| block_weight(b, col_scale)).sum();
            let rw: f32 = right.iter().map(|b| block_weight(b, col_scale)).sum();
            lw.max(rw)
        }
        // Slide renderers cap images at IMAGE_MAX_HEIGHT_FRACTION of
        // slide height (currently 65%) plus a caption line. After
        // subtracting chrome from the budget that works out to ~13
        // weight units on a 16:9 slide. Sized so an image plus a short
        // caption-style paragraph still triggers a split, keeping the
        // image on its own clean page. If you bump the renderer cap,
        // bump this value too.
        Block::Image { .. } => 13.0,
        Block::Footnotes(items) => {
            // Small text, so each line is ~half the weight of a normal list line.
            let total: f32 = items
                .iter()
                .map(|i| {
                    let chars = i.runs.iter().map(|r| r.text.chars().count()).sum::<usize>() as f32;
                    (chars / (110.0 * wscale)).max(0.7)
                })
                .sum();
            total + 0.4
        }
    }
}

/// A "lead-in" block introduces what follows — typically a short paragraph
/// ending in `:` (e.g. "Interpretation:", "How to use it:"). Detected so
/// pagination can avoid orphaning it at the bottom of a page.
fn is_lead_in(b: &Block) -> bool {
    match b {
        Block::Paragraph(runs) => {
            let text: String = runs.iter().map(|r| r.text.clone()).collect();
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return false;
            }
            let chars = trimmed.chars().count();
            chars < 120 && trimmed.ends_with(':')
        }
        Block::Heading { .. } => true,
        _ => false,
    }
}

fn coalesce_columns(blocks: Vec<Block>) -> Vec<Block> {
    if !blocks.iter().any(|b| matches!(b, Block::ColumnBreak)) {
        return blocks;
    }
    let mut left = Vec::new();
    let mut right = Vec::new();
    let mut break_seen = false;
    for b in blocks {
        match b {
            Block::ColumnBreak => {
                break_seen = true;
            }
            other if break_seen => right.push(other),
            other => left.push(other),
        }
    }
    vec![Block::Columns { left, right }]
}

fn total_chars(runs: &[Run]) -> usize {
    runs.iter().map(|r| r.text.chars().count()).sum()
}
