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
use crate::layout::{Layout, LayoutKind};
use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakMode {
    /// Preserve section/content slide structure, but do not split overlong content.
    Off,
    /// Existing block-level pagination: split code/tables, keep paragraphs/lists atomic.
    Simple,
    /// Split large paragraphs/lists/columns at readable boundaries.
    Smart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableFit {
    Auto,
    Split,
    Transpose,
    Off,
}

#[derive(Debug, Clone, Copy)]
pub struct PaginationOptions {
    pub break_mode: BreakMode,
    /// Fraction of the estimated content box to fill before breaking.
    /// Values below 1.0 make more, airier slides; values above 1.0 pack tighter.
    pub fill: f32,
    /// How wide tables are reshaped before slide pagination.
    pub table_fit: TableFit,
    /// Whether eligible landscape code blocks flow as two-up columns.
    pub code_columns: CodeColumns,
}

impl Default for PaginationOptions {
    fn default() -> Self {
        PaginationOptions {
            break_mode: BreakMode::Smart,
            fill: 1.0,
            table_fit: TableFit::Auto,
            code_columns: CodeColumns::Single,
        }
    }
}

impl PaginationOptions {
    pub fn with_fill_percent(mut self, pct: f32) -> Self {
        self.fill = (pct / 100.0).clamp(0.5, 1.2);
        self
    }

    fn effective_budget(self, budget: f32) -> f32 {
        if matches!(self.break_mode, BreakMode::Off) {
            budget
        } else {
            budget * self.fill.clamp(0.5, 1.2)
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ContentBox {
    pub x_emu: u32,
    pub y_emu: u32,
    pub w_emu: u32,
    pub h_emu: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct WeightContext {
    pub budget: f32,
    pub width_scale: f32,
    pub allow_auto_columns: bool,
}

/// Body-line budget for one slide content area.
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
    budget_for_content_height(content_h)
}

fn budget_for_content_height(content_h: f32) -> f32 {
    let chrome_emu = 1_828_800.0; // ≈ 2 in title + footer + padding
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

fn width_scale_for_content_width(content_w: u32) -> f32 {
    // The legacy divisors were tuned against a clean 16:9 slide's actual
    // content column, not the full slide canvas. Use that as the stable
    // reference point and let narrow frame/portrait layouts split earlier.
    let ref_w = 12_192_000.0 - 2.0 * 533_400.0;
    (content_w as f32 / ref_w).clamp(0.3, 1.2)
}

/// Code-block vertical weight per line, relative to one body line.
/// Code renders at ~14pt monospace with ~1.2 leading vs ~17pt body with
/// ~1.4 leading, so the rendered ratio is ~0.7. Anything materially lower
/// makes long code blocks overflow the bottom of 16:9 slides; anything
/// higher splits 30-line reference listings into near-empty pages on A4.
const CODE_LINE_WEIGHT: f32 = 0.7;

pub fn paginate(slides: Vec<Slide>, theme: &Theme) -> Vec<Slide> {
    paginate_with_options(slides, theme, PaginationOptions::default())
}

pub fn paginate_with_options(
    slides: Vec<Slide>,
    theme: &Theme,
    options: PaginationOptions,
) -> Vec<Slide> {
    paginate_inner(
        slides,
        theme,
        options.effective_budget(budget_for_theme(theme)),
        width_scale(theme),
        options,
    )
}

pub fn paginate_for_layout(slides: Vec<Slide>, theme: &Theme, layout: &Layout) -> Vec<Slide> {
    paginate_for_layout_with_options(slides, theme, layout, PaginationOptions::default())
}

pub fn paginate_for_layout_with_options(
    slides: Vec<Slide>,
    theme: &Theme,
    layout: &Layout,
    options: PaginationOptions,
) -> Vec<Slide> {
    let content = content_box(theme, layout);
    paginate_inner(
        slides,
        theme,
        options.effective_budget(budget_for_content_height(content.h_emu as f32)),
        width_scale_for_content_width(content.w_emu),
        options,
    )
}

pub fn weight_context_for_layout(
    theme: &Theme,
    layout: &Layout,
    options: PaginationOptions,
) -> WeightContext {
    let content = content_box(theme, layout);
    WeightContext {
        budget: options.effective_budget(budget_for_content_height(content.h_emu as f32)),
        width_scale: width_scale_for_content_width(content.w_emu),
        allow_auto_columns: !theme.portrait,
    }
}

pub fn estimate_block_weight(block: &Block, context: WeightContext) -> f32 {
    block_weight(block, context.width_scale, context.allow_auto_columns)
}

pub fn content_box(theme: &Theme, layout: &Layout) -> ContentBox {
    let w = theme.slide_w;
    let h = theme.slide_h;
    let base_margin: u32 = 533_400;
    let left_offset = layout.content_left_offset();
    let extra_left = if layout.shows_rail() { 200_000 } else { 0 };
    let content_x = if left_offset > 0 {
        left_offset + 480_000
    } else {
        base_margin + extra_left
    };
    let content_w = w.saturating_sub(content_x + base_margin);

    let title_y: u32 = if matches!(layout.kind, LayoutKind::Bold) {
        280_000
    } else {
        360_000
    };
    let title_h: u32 = if matches!(layout.kind, LayoutKind::Bold) {
        820_000
    } else {
        720_000
    };
    let underline_y = if layout.title_underline() {
        title_y + title_h + 30_000
    } else {
        title_y + title_h
    };
    let content_y_start = underline_y
        + if layout.title_underline() {
            200_000
        } else {
            280_000
        };
    let footer_y = h.saturating_sub(400_000);
    let content_max_y = footer_y.saturating_sub(100_000);
    let content_h = content_max_y.saturating_sub(content_y_start);
    ContentBox {
        x_emu: content_x,
        y_emu: content_y_start,
        w_emu: content_w,
        h_emu: content_h,
    }
}

fn paginate_inner(
    slides: Vec<Slide>,
    theme: &Theme,
    budget: f32,
    wscale: f32,
    options: PaginationOptions,
) -> Vec<Slide> {
    let allow_auto_columns = !theme.portrait;
    let mut out = Vec::new();
    for mut slide in slides {
        slide.blocks = fit_tables(std::mem::take(&mut slide.blocks), theme, options.table_fit);
        slide.blocks = coalesce_columns(std::mem::take(&mut slide.blocks));
        if slide.layout_hint.as_deref() != Some("text-full") {
            slide.blocks = fit_code_columns(std::mem::take(&mut slide.blocks), theme, options);
        }

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
                for s in paginate_inner(vec![follow], theme, budget, wscale, options) {
                    out.push(s);
                }
            }
            continue;
        }
        if !matches!(slide.kind, SlideKind::Content) {
            out.push(slide);
            continue;
        }
        if matches!(options.break_mode, BreakMode::Off) {
            out.push(slide);
            continue;
        }
        let total: f32 = slide
            .blocks
            .iter()
            .map(|b| block_weight(b, wscale, allow_auto_columns))
            .sum();
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

        for block in std::mem::take(&mut slide.blocks) {
            let w = block_weight(&block, wscale, allow_auto_columns);
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
                    weight += block_weight(&lead, wscale, allow_auto_columns);
                    current.blocks.push(lead);
                }
            }

            if matches!(&block, Block::Paragraph(runs) if is_smart(options) && block_weight(&block, wscale, allow_auto_columns) > budget + 0.5 && runs_text_len(runs) > 1)
            {
                if let Block::Paragraph(runs) = block {
                    let chunks = split_paragraph_runs(runs, budget, wscale);
                    for chunk in chunks {
                        let chunk_block = Block::Paragraph(chunk);
                        let chunk_weight = block_weight(&chunk_block, wscale, allow_auto_columns);
                        if !current.blocks.is_empty() && weight + chunk_weight > budget {
                            out.push(current);
                            emitted_first = true;
                            current = continuation_slide(&slide, emitted_first);
                            weight = 0.0;
                        }
                        current.blocks.push(chunk_block);
                        weight += chunk_weight;
                    }
                }
                continue;
            }

            if matches!(&block, Block::List(items) if is_smart(options) && list_weight(items, wscale, allow_auto_columns) > budget + 0.5 && items.len() > 1)
            {
                if let Block::List(items) = block {
                    let chunks = split_list_items(items, budget, wscale, allow_auto_columns);
                    for chunk in chunks {
                        let chunk_block = Block::List(chunk);
                        let chunk_weight = block_weight(&chunk_block, wscale, allow_auto_columns);
                        if !current.blocks.is_empty() && weight + chunk_weight > budget {
                            out.push(current);
                            emitted_first = true;
                            current = continuation_slide(&slide, emitted_first);
                            weight = 0.0;
                        }
                        current.blocks.push(chunk_block);
                        weight += chunk_weight;
                    }
                }
                continue;
            }

            if matches!(&block, Block::Columns { .. } if is_smart(options) && block_weight(&block, wscale, allow_auto_columns) > budget + 0.5)
            {
                if let Block::Columns { left, right } = block {
                    let col_scale = wscale * 0.5;
                    let chunks =
                        split_columns(left, right, budget, col_scale, allow_auto_columns, options);
                    for chunk_block in chunks {
                        let chunk_weight = block_weight(&chunk_block, wscale, allow_auto_columns);
                        if !current.blocks.is_empty() && weight + chunk_weight > budget {
                            out.push(current);
                            emitted_first = true;
                            current = continuation_slide(&slide, emitted_first);
                            weight = 0.0;
                        }
                        current.blocks.push(chunk_block);
                        weight += chunk_weight;
                    }
                }
                continue;
            }

            if matches!(&block, Block::CodeBlock { lines, title, lang, .. } if code_block_weight(lines.len(), title, lang) > budget + 2.0)
            {
                if let Block::CodeBlock {
                    lang,
                    title,
                    lines,
                    line_numbers,
                    start_line,
                    columns,
                    include_error,
                } = block
                {
                    let chunk_size = ((budget / CODE_LINE_WEIGHT) as usize)
                        .saturating_sub(2)
                        .max(6);
                    let chunks = if is_smart(options) {
                        split_code_lines(lines, chunk_size)
                    } else {
                        lines
                            .chunks(chunk_size)
                            .map(|chunk| chunk.to_vec())
                            .collect()
                    };
                    let mut next_start_line = start_line;
                    for (idx, chunk) in chunks.into_iter().enumerate() {
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
                        let chunk_start_line = next_start_line;
                        next_start_line += chunk.len();
                        let chunk_block = Block::CodeBlock {
                            lang: lang.clone(),
                            title: if idx == 0 { title.clone() } else { None },
                            lines: chunk,
                            line_numbers,
                            start_line: chunk_start_line,
                            columns,
                            include_error: include_error.clone(),
                        };
                        weight += block_weight(&chunk_block, wscale, allow_auto_columns);
                        current.blocks.push(chunk_block);
                    }
                }
                continue;
            }

            // Mirror of the code-block branch for GFM tables. The header
            // row is repeated on each continuation slide so a reader who
            // lands on page N still sees the column titles.
            if matches!(&block, Block::Table { headers, rows } if table_weight(headers, rows, wscale) > budget + 1.0)
            {
                if let Block::Table { headers, rows } = block {
                    let chunks = split_table_rows(headers.as_slice(), rows, budget, wscale);
                    for (idx, chunk) in chunks.into_iter().enumerate() {
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
                        let chunk_weight = table_weight(&headers, &chunk, wscale);
                        current.blocks.push(Block::Table {
                            headers: headers.clone(),
                            rows: chunk,
                        });
                        weight += chunk_weight;
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

fn fit_tables(blocks: Vec<Block>, theme: &Theme, mode: TableFit) -> Vec<Block> {
    let mut out = Vec::new();
    for block in blocks {
        match block {
            Block::Table { headers, rows } => {
                out.extend(fit_table(headers, rows, theme, mode));
            }
            Block::Columns { left, right } => out.push(Block::Columns {
                left: fit_tables(left, theme, mode),
                right: fit_tables(right, theme, mode),
            }),
            other => out.push(other),
        }
    }
    out
}

fn fit_code_columns(blocks: Vec<Block>, theme: &Theme, options: PaginationOptions) -> Vec<Block> {
    let mut out = Vec::new();
    for block in blocks {
        match block {
            Block::CodeBlock {
                lang,
                title,
                mut lines,
                line_numbers,
                start_line,
                columns,
                include_error,
            } => {
                let mode = columns.unwrap_or(options.code_columns);
                if !should_code_two_up(mode, &lines, theme) || include_error.is_some() {
                    out.push(Block::CodeBlock {
                        lang,
                        title,
                        lines,
                        line_numbers,
                        start_line,
                        columns,
                        include_error,
                    });
                    continue;
                }

                let left_len = lines.len().div_ceil(2);
                let right_lines = lines.split_off(left_len);
                let right_start_line = start_line + left_len;
                let left = Block::CodeBlock {
                    lang: lang.clone(),
                    title: title.clone(),
                    lines,
                    line_numbers,
                    start_line,
                    columns: Some(CodeColumns::Single),
                    include_error: None,
                };
                let right = Block::CodeBlock {
                    lang,
                    title: title.map(|t| format!("{t} (cont.)")),
                    lines: right_lines,
                    line_numbers,
                    start_line: right_start_line,
                    columns: Some(CodeColumns::Single),
                    include_error: None,
                };
                out.push(Block::Columns {
                    left: vec![left],
                    right: vec![right],
                });
            }
            other => out.push(other),
        }
    }
    out
}

fn should_code_two_up(mode: CodeColumns, lines: &[String], theme: &Theme) -> bool {
    if theme.portrait || lines.len() < 8 {
        return false;
    }
    match mode {
        CodeColumns::Single => false,
        CodeColumns::TwoUp => true,
        CodeColumns::Auto => {
            let max_chars = lines
                .iter()
                .map(|line| line.chars().count())
                .max()
                .unwrap_or(0);
            let avg_chars = if lines.is_empty() {
                0.0
            } else {
                lines.iter().map(|line| line.chars().count()).sum::<usize>() as f32
                    / lines.len() as f32
            };
            lines.len() >= 18 && max_chars <= 88 && avg_chars <= 62.0
        }
    }
}

fn fit_table(
    headers: Vec<Vec<Run>>,
    rows: Vec<Vec<Vec<Run>>>,
    theme: &Theme,
    mode: TableFit,
) -> Vec<Block> {
    if matches!(mode, TableFit::Off) {
        return vec![Block::Table { headers, rows }];
    }
    let cols = table_col_count(&headers, &rows);
    if cols <= 1 {
        return vec![Block::Table { headers, rows }];
    }
    let max_cols = if theme.portrait { 4 } else { 7 };
    if cols <= max_cols {
        return vec![Block::Table { headers, rows }];
    }

    let use_transpose = match mode {
        TableFit::Transpose => true,
        TableFit::Auto => theme.portrait && rows.len() <= 3,
        TableFit::Split | TableFit::Off => false,
    };
    if use_transpose {
        return vec![transpose_table(headers, rows, cols)];
    }
    split_table_columns(headers, rows, cols, max_cols)
}

fn table_col_count(headers: &[Vec<Run>], rows: &[Vec<Vec<Run>>]) -> usize {
    headers
        .len()
        .max(rows.iter().map(|row| row.len()).max().unwrap_or(0))
}

fn transpose_table(headers: Vec<Vec<Run>>, rows: Vec<Vec<Vec<Run>>>, cols: usize) -> Block {
    let mut out_headers = Vec::with_capacity(rows.len() + 1);
    out_headers.push(vec![Run::plain("Field")]);
    for row_idx in 0..rows.len() {
        out_headers.push(vec![Run::plain(format!("Row {}", row_idx + 1))]);
    }

    let mut out_rows = Vec::with_capacity(cols);
    for col_idx in 0..cols {
        let mut row = Vec::with_capacity(rows.len() + 1);
        row.push(table_cell(&headers, col_idx));
        for source_row in &rows {
            row.push(source_row.get(col_idx).cloned().unwrap_or_default());
        }
        out_rows.push(row);
    }
    Block::Table {
        headers: out_headers,
        rows: out_rows,
    }
}

fn split_table_columns(
    headers: Vec<Vec<Run>>,
    rows: Vec<Vec<Vec<Run>>>,
    cols: usize,
    max_cols: usize,
) -> Vec<Block> {
    let group_width = max_cols.saturating_sub(1).max(1);
    let mut out = Vec::new();
    let mut start = 1usize;
    while start < cols {
        let end = (start + group_width).min(cols);
        let mut col_indices = Vec::with_capacity(end - start + 1);
        col_indices.push(0);
        col_indices.extend(start..end);
        out.push(Block::Table {
            headers: pick_header_cells(&headers, &col_indices),
            rows: rows
                .iter()
                .map(|row| pick_row_cells(row, &col_indices))
                .collect(),
        });
        start = end;
    }
    if out.is_empty() {
        out.push(Block::Table { headers, rows });
    }
    out
}

fn pick_header_cells(headers: &[Vec<Run>], indices: &[usize]) -> Vec<Vec<Run>> {
    indices
        .iter()
        .map(|idx| table_cell(headers, *idx))
        .collect()
}

fn pick_row_cells(row: &[Vec<Run>], indices: &[usize]) -> Vec<Vec<Run>> {
    indices
        .iter()
        .map(|idx| row.get(*idx).cloned().unwrap_or_default())
        .collect()
}

fn table_cell(cells: &[Vec<Run>], idx: usize) -> Vec<Run> {
    cells.get(idx).cloned().unwrap_or_default()
}

fn split_table_rows(
    headers: &[Vec<Run>],
    rows: Vec<Vec<Vec<Run>>>,
    budget: f32,
    wscale: f32,
) -> Vec<Vec<Vec<Vec<Run>>>> {
    let cols = table_col_count(headers, &rows);
    let header_weight = table_row_weight(headers, cols, wscale).max(1.2);
    let row_budget = (budget - header_weight).max(2.0);
    let mut chunks = Vec::new();
    let mut chunk = Vec::new();
    let mut weight = 0.0_f32;

    for row in rows {
        let row_weight = table_row_weight(&row, cols, wscale);
        if !chunk.is_empty() && weight + row_weight > row_budget {
            chunks.push(chunk);
            chunk = Vec::new();
            weight = 0.0;
        }
        weight += row_weight;
        chunk.push(row);
    }

    if !chunk.is_empty() {
        chunks.push(chunk);
    }
    chunks
}

fn table_weight(headers: &[Vec<Run>], rows: &[Vec<Vec<Run>>], wscale: f32) -> f32 {
    let cols = table_col_count(headers, rows);
    table_row_weight(headers, cols, wscale).max(1.2)
        + rows
            .iter()
            .map(|row| table_row_weight(row, cols, wscale))
            .sum::<f32>()
}

fn table_row_weight(cells: &[Vec<Run>], cols: usize, wscale: f32) -> f32 {
    let cols = cols.max(1) as f32;
    let chars_per_col = (80.0 * wscale / cols).clamp(14.0, 90.0);
    let lines = cells
        .iter()
        .map(|cell| (runs_text_len(cell) as f32 / chars_per_col).ceil().max(1.0))
        .fold(1.0_f32, f32::max);
    0.45 + lines * 0.95
}

fn block_weight(b: &Block, wscale: f32, allow_auto_columns: bool) -> f32 {
    match b {
        Block::Paragraph(runs) => {
            let chars = total_chars(runs) as f32;
            (chars / (85.0 * wscale)).max(1.0) + 0.4
        }
        Block::Heading { .. } => 1.6,
        Block::List(items) => list_weight(items, wscale, allow_auto_columns),
        Block::CodeBlock {
            lines, title, lang, ..
        } => code_block_weight(lines.len(), title, lang),
        Block::Quote(paras) => {
            paras
                .iter()
                .map(|runs| (total_chars(runs) as f32 / (70.0 * wscale)).max(1.0))
                .sum::<f32>()
                + 0.6
        }
        Block::Table { headers, rows } => table_weight(headers, rows, wscale),
        Block::ColumnBreak => 0.0,
        Block::Columns { left, right } => {
            // Two-column layout halves the per-column width, so each side
            // wraps to roughly twice as many lines as it would full-width.
            let col_scale = wscale * 0.5;
            let lw: f32 = left
                .iter()
                .map(|b| block_weight(b, col_scale, allow_auto_columns))
                .sum();
            let rw: f32 = right
                .iter()
                .map(|b| block_weight(b, col_scale, allow_auto_columns))
                .sum();
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

fn code_block_weight(lines: usize, title: &Option<String>, lang: &Option<String>) -> f32 {
    let has_header = title.is_some() || lang.is_some();
    lines as f32 * CODE_LINE_WEIGHT + 1.5 + if has_header { 0.9 } else { 0.0 }
}

fn is_smart(options: PaginationOptions) -> bool {
    matches!(options.break_mode, BreakMode::Smart)
}

fn continuation_slide(source: &Slide, emitted_first: bool) -> Slide {
    Slide {
        kind: SlideKind::Content,
        title: if emitted_first && !source.title.ends_with(" (cont.)") {
            format!("{} (cont.)", source.title)
        } else {
            source.title.clone()
        },
        blocks: Vec::new(),
        notes: None,
        bg_image: source.bg_image.clone(),
        layout_hint: source.layout_hint.clone(),
    }
}

fn runs_text_len(runs: &[Run]) -> usize {
    runs.iter().map(|r| r.text.chars().count()).sum()
}

fn split_paragraph_runs(runs: Vec<Run>, budget: f32, wscale: f32) -> Vec<Vec<Run>> {
    let total_chars = runs_text_len(&runs);
    let max_chars = ((budget - 0.4).max(1.0) * 85.0 * wscale).floor().max(40.0) as usize;
    if total_chars <= max_chars {
        return vec![runs];
    }

    let text = runs_text(&runs);
    let offsets = text_chunk_offsets(&text, max_chars);
    split_runs_at_offsets(runs, &offsets)
}

fn text_chunk_offsets(text: &str, max_chars: usize) -> Vec<usize> {
    let chars: Vec<char> = text.chars().collect();
    let mut offsets = Vec::new();
    let mut start = 0usize;
    while start + max_chars < chars.len() {
        let limit = (start + max_chars).min(chars.len());
        let min_break = start + (max_chars / 3).max(20).min(max_chars);
        let split = find_break(&chars, start, min_break, limit).unwrap_or(limit);
        if split <= start {
            break;
        }
        offsets.push(split);
        start = split;
        while start < chars.len() && chars[start].is_whitespace() {
            start += 1;
        }
        if let Some(last) = offsets.last_mut() {
            *last = start;
        }
    }
    offsets.retain(|offset| *offset > 0 && *offset < chars.len());
    offsets.dedup();
    offsets
}

fn find_break(chars: &[char], start: usize, min_break: usize, limit: usize) -> Option<usize> {
    for i in (min_break..limit).rev() {
        let c = chars[i];
        if matches!(c, '.' | '!' | '?') {
            return Some((i + 1).min(chars.len()));
        }
    }
    for i in (min_break..limit).rev() {
        let c = chars[i];
        if matches!(c, ';' | ':' | ',') {
            return Some((i + 1).min(chars.len()));
        }
    }
    for i in (min_break..limit).rev() {
        if chars[i].is_whitespace() {
            return Some(i + 1);
        }
    }
    if limit > start {
        Some(limit)
    } else {
        None
    }
}

fn split_runs_at_offsets(runs: Vec<Run>, offsets: &[usize]) -> Vec<Vec<Run>> {
    if offsets.is_empty() {
        return vec![runs];
    }
    let mut chunks: Vec<Vec<Run>> = Vec::new();
    let mut current: Vec<Run> = Vec::new();
    let mut next_offset_idx = 0usize;
    let mut seen = 0usize;

    for run in runs {
        let mut text_start = 0usize;
        let run_chars: Vec<char> = run.text.chars().collect();
        while next_offset_idx < offsets.len() {
            let offset = offsets[next_offset_idx];
            if offset <= seen || offset >= seen + run_chars.len() {
                break;
            }
            let split_at = offset - seen;
            push_run_piece(&mut current, &run, &run_chars[text_start..split_at]);
            trim_run_edges(&mut current);
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            text_start = split_at;
            next_offset_idx += 1;
        }
        push_run_piece(&mut current, &run, &run_chars[text_start..]);
        seen += run_chars.len();
        while next_offset_idx < offsets.len() && offsets[next_offset_idx] <= seen {
            trim_run_edges(&mut current);
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            next_offset_idx += 1;
        }
    }

    trim_run_edges(&mut current);
    if !current.is_empty() {
        chunks.push(current);
    }
    if chunks.is_empty() {
        vec![Vec::new()]
    } else {
        chunks
    }
}

fn push_run_piece(out: &mut Vec<Run>, template: &Run, chars: &[char]) {
    if chars.is_empty() {
        return;
    }
    let text: String = chars.iter().collect();
    if text.is_empty() {
        return;
    }
    let mut run = template.clone();
    run.text = text;
    out.push(run);
}

fn trim_run_edges(runs: &mut Vec<Run>) {
    while runs.first().map_or(false, |r| r.text.trim().is_empty()) {
        runs.remove(0);
    }
    while runs.last().map_or(false, |r| r.text.trim().is_empty()) {
        runs.pop();
    }
    if let Some(first) = runs.first_mut() {
        first.text = first.text.trim_start().to_string();
    }
    if let Some(last) = runs.last_mut() {
        last.text = last.text.trim_end().to_string();
    }
}

fn list_weight(items: &[ListItem], wscale: f32, allow_auto_columns: bool) -> f32 {
    let total: f32 = items.iter().map(|i| list_item_weight(i, wscale)).sum();
    // Long lists of short bullets render denser than the per-item
    // calculation suggests only when the renderer actually uses two
    // columns. Portrait decks stay single-column, so discounting them here
    // lets bullets run through the footer.
    if allow_auto_columns
        && items.len() > crate::theme::LONG_LIST_THRESHOLD
        && all_list_items_short(items, wscale)
    {
        (total / 2.0).max(items.len() as f32 / 2.0)
    } else {
        total
    }
}

fn list_item_weight(item: &ListItem, wscale: f32) -> f32 {
    let chars = item
        .runs
        .iter()
        .map(|r| r.text.chars().count())
        .sum::<usize>() as f32;
    let lines = (chars / (75.0 * wscale)).max(1.0);
    lines + 0.1 * item.level as f32
}

fn all_list_items_short(items: &[ListItem], wscale: f32) -> bool {
    items.iter().all(|i| {
        let chars = i.runs.iter().map(|r| r.text.chars().count()).sum::<usize>() as f32;
        chars <= 75.0 * wscale
    })
}

fn split_list_items(
    items: Vec<ListItem>,
    budget: f32,
    wscale: f32,
    allow_auto_columns: bool,
) -> Vec<Vec<ListItem>> {
    if items.is_empty() {
        return Vec::new();
    }

    if allow_auto_columns
        && items.len() > crate::theme::LONG_LIST_THRESHOLD
        && all_list_items_short(&items, wscale)
    {
        let max_items = ((budget * 2.0).floor() as usize)
            .max(crate::theme::LONG_LIST_THRESHOLD + 1)
            .max(1);
        return items
            .chunks(max_items)
            .map(|chunk| chunk.to_vec())
            .collect();
    }

    let mut chunks: Vec<Vec<ListItem>> = Vec::new();
    let mut current: Vec<ListItem> = Vec::new();
    for item in items {
        if current.is_empty() {
            current.push(item);
            continue;
        }
        let mut candidate = current.clone();
        candidate.push(item.clone());
        if list_weight(&candidate, wscale, allow_auto_columns) > budget {
            chunks.push(std::mem::take(&mut current));
            current.push(item);
        } else {
            current.push(item);
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn split_columns(
    left: Vec<Block>,
    right: Vec<Block>,
    budget: f32,
    col_scale: f32,
    allow_auto_columns: bool,
    options: PaginationOptions,
) -> Vec<Block> {
    let left_chunks = split_blocks_to_fit(left, budget, col_scale, allow_auto_columns, options);
    let right_chunks = split_blocks_to_fit(right, budget, col_scale, allow_auto_columns, options);
    let n = left_chunks.len().max(right_chunks.len()).max(1);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        out.push(Block::Columns {
            left: left_chunks.get(i).cloned().unwrap_or_default(),
            right: right_chunks.get(i).cloned().unwrap_or_default(),
        });
    }
    out
}

fn split_blocks_to_fit(
    blocks: Vec<Block>,
    budget: f32,
    wscale: f32,
    allow_auto_columns: bool,
    options: PaginationOptions,
) -> Vec<Vec<Block>> {
    let mut chunks: Vec<Vec<Block>> = Vec::new();
    let mut current: Vec<Block> = Vec::new();
    let mut weight = 0.0_f32;

    for block in blocks {
        let expanded = split_block_if_needed(block, budget, wscale, allow_auto_columns, options);
        for part in expanded {
            let part_weight = block_weight(&part, wscale, allow_auto_columns);
            if !current.is_empty() && weight + part_weight > budget {
                chunks.push(std::mem::take(&mut current));
                weight = 0.0;
            }
            current.push(part);
            weight += part_weight;
        }
    }

    if !current.is_empty() {
        chunks.push(current);
    }
    if chunks.is_empty() {
        vec![Vec::new()]
    } else {
        chunks
    }
}

fn split_block_if_needed(
    block: Block,
    budget: f32,
    wscale: f32,
    allow_auto_columns: bool,
    options: PaginationOptions,
) -> Vec<Block> {
    if !is_smart(options) || block_weight(&block, wscale, allow_auto_columns) <= budget + 0.5 {
        return vec![block];
    }
    match block {
        Block::Paragraph(runs) => split_paragraph_runs(runs, budget, wscale)
            .into_iter()
            .map(Block::Paragraph)
            .collect(),
        Block::List(items) if items.len() > 1 => {
            split_list_items(items, budget, wscale, allow_auto_columns)
                .into_iter()
                .map(Block::List)
                .collect()
        }
        Block::Columns { left, right } => split_columns(
            left,
            right,
            budget,
            wscale * 0.5,
            allow_auto_columns,
            options,
        ),
        other => vec![other],
    }
}

fn split_code_lines(lines: Vec<String>, chunk_size: usize) -> Vec<Vec<String>> {
    if lines.len() <= chunk_size {
        return vec![lines];
    }
    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < lines.len() {
        let limit = (start + chunk_size).min(lines.len());
        if limit == lines.len() {
            chunks.push(lines[start..].to_vec());
            break;
        }
        let min_break = start + (chunk_size * 2 / 3).max(1);
        let split = find_code_break(&lines, min_break, limit).unwrap_or(limit);
        let split = split.max(start + 1).min(lines.len());
        chunks.push(lines[start..split].to_vec());
        start = split;
    }
    chunks
}

fn find_code_break(lines: &[String], min_break: usize, limit: usize) -> Option<usize> {
    for i in (min_break..limit).rev() {
        if lines[i].trim().is_empty() {
            return Some(i + 1);
        }
    }
    for i in (min_break..limit).rev() {
        let trimmed = lines[i].trim_start();
        if trimmed.starts_with("fn ")
            || trimmed.starts_with("def ")
            || trimmed.starts_with("class ")
            || trimmed.starts_with("impl ")
            || trimmed.starts_with("func ")
            || trimmed.starts_with("function ")
            || trimmed.starts_with("SELECT ")
            || trimmed.starts_with("WITH ")
        {
            return Some(i);
        }
    }
    None
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
