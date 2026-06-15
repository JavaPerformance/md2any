//! JSON diagnostics for the post-parse/post-pagination deck.
//!
//! This module is the first shared render-plan layer. The current writers
//! still render from the IR directly, but this plan records the geometry and
//! weight estimates they should converge on. It is intentionally serializable
//! and stable enough for bug reports, CI artifacts, and future renderer tests.

use crate::ir::*;
use crate::layout::Layout;
use crate::paginate::{self, BreakMode, PaginationOptions, TableFit, WeightContext};
use crate::theme::Theme;
use serde::Serialize;

const SCHEMA_IR: &str = "md2any-ir-v1";
const SCHEMA_PLAN: &str = "md2any-render-plan-v1";
const EMU_PER_PT: f32 = 12_700.0;

#[derive(Debug, Serialize)]
pub struct IrDump {
    pub schema: String,
    pub slide_count: usize,
    pub slides: Vec<IrSlide>,
}

#[derive(Debug, Serialize)]
pub struct IrSlide {
    pub index: usize,
    pub kind: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bg_image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout_hint: Option<String>,
    pub blocks: Vec<IrBlock>,
}

#[derive(Debug, Serialize)]
pub struct IrBlock {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<u8>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub items: Vec<IrListItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub lines: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_numbers: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columns: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_error: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub headers: Vec<Vec<String>>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub rows: Vec<Vec<Vec<String>>>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub left: Vec<IrBlock>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub right: Vec<IrBlock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub src: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width_pct: Option<u8>,
}

#[derive(Debug, Serialize)]
pub struct IrListItem {
    pub text: String,
    pub level: u8,
    pub ordered: bool,
}

#[derive(Debug, Serialize)]
pub struct RenderPlan {
    pub schema: String,
    pub input: String,
    pub deck_title: String,
    pub theme: ThemePlan,
    pub layout: LayoutPlan,
    pub pagination: PaginationPlan,
    pub slide_count: usize,
    pub slides: Vec<SlidePlan>,
}

#[derive(Debug, Serialize)]
pub struct ThemePlan {
    pub name: String,
    pub aspect_kind: String,
    pub portrait: bool,
    pub slide_emu: SizePlan,
    pub page_pt: SizePlan,
}

#[derive(Debug, Serialize)]
pub struct LayoutPlan {
    pub name: String,
    pub content_box: BoxPlan,
}

#[derive(Debug, Serialize)]
pub struct PaginationPlan {
    pub break_mode: String,
    pub table_fit: String,
    pub code_columns: String,
    pub fill: f32,
    pub budget_weight: f32,
    pub width_scale: f32,
    pub allow_auto_columns: bool,
}

#[derive(Debug, Serialize)]
pub struct SizePlan {
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, Serialize)]
pub struct BoxPlan {
    pub x_emu: u32,
    pub y_emu: u32,
    pub w_emu: u32,
    pub h_emu: u32,
    pub x_pt: f32,
    pub y_pt: f32,
    pub w_pt: f32,
    pub h_pt: f32,
}

#[derive(Debug, Serialize)]
pub struct SlidePlan {
    pub index: usize,
    pub kind: String,
    pub title: String,
    pub continuation: bool,
    pub has_notes: bool,
    pub has_background: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout_hint: Option<String>,
    pub estimated_weight: f32,
    pub budget_weight: f32,
    pub fill_ratio: f32,
    pub blocks: Vec<BlockPlan>,
}

#[derive(Debug, Serialize)]
pub struct BlockPlan {
    pub index: usize,
    pub path: String,
    pub kind: String,
    pub summary: String,
    pub estimated_weight: f32,
    pub metrics: BlockMetrics,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub children: Vec<BlockPlan>,
}

#[derive(Debug, Default, Serialize)]
pub struct BlockMetrics {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chars: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columns: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_src: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width_pct: Option<u8>,
}

pub fn ir_dump(slides: &[Slide]) -> IrDump {
    IrDump {
        schema: SCHEMA_IR.to_string(),
        slide_count: slides.len(),
        slides: slides
            .iter()
            .enumerate()
            .map(|(idx, slide)| IrSlide {
                index: idx + 1,
                kind: slide_kind_name(&slide.kind).to_string(),
                title: slide.title.clone(),
                notes: slide.notes.clone(),
                bg_image: slide.bg_image.clone(),
                layout_hint: slide.layout_hint.clone(),
                blocks: slide.blocks.iter().map(ir_block).collect(),
            })
            .collect(),
    }
}

pub fn build(
    input: impl Into<String>,
    deck_title: impl Into<String>,
    slides: &[Slide],
    theme: &Theme,
    layout: &Layout,
    options: PaginationOptions,
) -> RenderPlan {
    let content = paginate::content_box(theme, layout);
    let context = paginate::weight_context_for_layout(theme, layout, options);
    let content_box = BoxPlan {
        x_emu: content.x_emu,
        y_emu: content.y_emu,
        w_emu: content.w_emu,
        h_emu: content.h_emu,
        x_pt: emu_to_pt(content.x_emu),
        y_pt: emu_to_pt(content.y_emu),
        w_pt: emu_to_pt(content.w_emu),
        h_pt: emu_to_pt(content.h_emu),
    };

    RenderPlan {
        schema: SCHEMA_PLAN.to_string(),
        input: input.into(),
        deck_title: deck_title.into(),
        theme: ThemePlan {
            name: theme.name.to_string(),
            aspect_kind: theme.aspect_kind.to_string(),
            portrait: theme.portrait,
            slide_emu: SizePlan {
                w: theme.slide_w,
                h: theme.slide_h,
            },
            page_pt: SizePlan {
                w: emu_to_pt(theme.slide_w) as u32,
                h: emu_to_pt(theme.slide_h) as u32,
            },
        },
        layout: LayoutPlan {
            name: layout.name().to_string(),
            content_box,
        },
        pagination: PaginationPlan {
            break_mode: break_mode_name(options.break_mode).to_string(),
            table_fit: table_fit_name(options.table_fit).to_string(),
            code_columns: code_columns_name(options.code_columns).to_string(),
            fill: round3(options.fill),
            budget_weight: round3(context.budget),
            width_scale: round3(context.width_scale),
            allow_auto_columns: context.allow_auto_columns,
        },
        slide_count: slides.len(),
        slides: slides
            .iter()
            .enumerate()
            .map(|(idx, slide)| slide_plan(idx, slide, context))
            .collect(),
    }
}

pub fn trace_text(plan: &RenderPlan) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "md2any render plan: {} slide(s), theme={}, layout={}, page={}x{}pt, content={}x{}pt, break={} table_fit={} code_columns={} fill={:.0}% budget={:.2}\n",
        plan.slide_count,
        plan.theme.name,
        plan.layout.name,
        plan.theme.page_pt.w,
        plan.theme.page_pt.h,
        plan.layout.content_box.w_pt,
        plan.layout.content_box.h_pt,
        plan.pagination.break_mode,
        plan.pagination.table_fit,
        plan.pagination.code_columns,
        plan.pagination.fill * 100.0,
        plan.pagination.budget_weight,
    ));
    for slide in &plan.slides {
        out.push_str(&format!(
            "{:>3}/{:<3} {:<7} {:>5.1}% {:>5.2}/{:<5.2} {}\n",
            slide.index,
            plan.slide_count,
            slide.kind,
            slide.fill_ratio * 100.0,
            slide.estimated_weight,
            slide.budget_weight,
            slide.title,
        ));
        for block in &slide.blocks {
            out.push_str(&format!(
                "      {:<11} w={:<5.2} {}\n",
                block.kind, block.estimated_weight, block.summary
            ));
        }
    }
    out
}

fn slide_plan(idx: usize, slide: &Slide, context: WeightContext) -> SlidePlan {
    let blocks: Vec<BlockPlan> = slide
        .blocks
        .iter()
        .enumerate()
        .map(|(block_idx, block)| block_plan(block_idx, block, context, block_idx.to_string()))
        .collect();
    let estimated_weight = blocks.iter().map(|b| b.estimated_weight).sum::<f32>();
    SlidePlan {
        index: idx + 1,
        kind: slide_kind_name(&slide.kind).to_string(),
        title: slide.title.clone(),
        continuation: slide.title.ends_with(" (cont.)"),
        has_notes: slide.notes.is_some(),
        has_background: slide.bg_image.is_some(),
        layout_hint: slide.layout_hint.clone(),
        estimated_weight: round3(estimated_weight),
        budget_weight: round3(context.budget),
        fill_ratio: if context.budget > 0.0 {
            round3(estimated_weight / context.budget)
        } else {
            0.0
        },
        blocks,
    }
}

fn block_plan(index: usize, block: &Block, context: WeightContext, path: String) -> BlockPlan {
    let estimated_weight = paginate::estimate_block_weight(block, context);
    let (kind, summary, metrics, children) = match block {
        Block::Paragraph(runs) => {
            let text = runs_text(runs);
            let chars = text.chars().count();
            (
                "paragraph",
                truncate(&text, 96),
                BlockMetrics {
                    chars: Some(chars),
                    ..Default::default()
                },
                Vec::new(),
            )
        }
        Block::Heading { level, runs } => {
            let text = runs_text(runs);
            (
                "heading",
                format!("h{} {}", level, truncate(&text, 88)),
                BlockMetrics {
                    chars: Some(text.chars().count()),
                    ..Default::default()
                },
                Vec::new(),
            )
        }
        Block::List(items) => (
            "list",
            format!("{} item(s)", items.len()),
            BlockMetrics {
                items: Some(items.len()),
                chars: Some(
                    items
                        .iter()
                        .map(|item| runs_text(&item.runs).chars().count())
                        .sum(),
                ),
                ..Default::default()
            },
            Vec::new(),
        ),
        Block::CodeBlock {
            lang,
            title,
            lines,
            start_line,
            ..
        } => {
            let end_line = start_line.saturating_add(lines.len().saturating_sub(1));
            let label = match (lang.as_deref(), title.as_deref()) {
                (Some(lang), Some(title)) => format!("{lang} {title}: "),
                (Some(lang), None) => format!("{lang}: "),
                (None, Some(title)) => format!("{title}: "),
                (None, None) => String::new(),
            };
            (
                "code",
                format!(
                    "{}{} line(s), source {}-{}",
                    label,
                    lines.len(),
                    start_line,
                    end_line,
                ),
                BlockMetrics {
                    lines: Some(lines.len()),
                    start_line: Some(*start_line),
                    end_line: Some(end_line),
                    chars: Some(lines.iter().map(|line| line.chars().count()).sum()),
                    ..Default::default()
                },
                Vec::new(),
            )
        }
        Block::Quote(paras) | Block::Callout { body: paras, .. } => {
            let chars = paras
                .iter()
                .map(|runs| runs_text(runs).chars().count())
                .sum();
            (
                "quote",
                format!("{} paragraph(s)", paras.len()),
                BlockMetrics {
                    chars: Some(chars),
                    ..Default::default()
                },
                Vec::new(),
            )
        }
        Block::Table { headers, rows, .. } => (
            "table",
            format!("{} column(s), {} row(s)", headers.len(), rows.len()),
            BlockMetrics {
                columns: Some(headers.len()),
                rows: Some(rows.len()),
                ..Default::default()
            },
            Vec::new(),
        ),
        Block::ColumnBreak => (
            "column_break",
            "column break marker".to_string(),
            BlockMetrics::default(),
            Vec::new(),
        ),
        Block::Columns { left, right } => {
            let child_context = WeightContext {
                width_scale: context.width_scale * 0.5,
                ..context
            };
            let mut children = Vec::new();
            for (idx, child) in left.iter().enumerate() {
                children.push(block_plan(
                    idx,
                    child,
                    child_context,
                    format!("{path}.left.{idx}"),
                ));
            }
            for (idx, child) in right.iter().enumerate() {
                children.push(block_plan(
                    idx,
                    child,
                    child_context,
                    format!("{path}.right.{idx}"),
                ));
            }
            (
                "columns",
                format!(
                    "left {} block(s), right {} block(s)",
                    left.len(),
                    right.len()
                ),
                BlockMetrics {
                    columns: Some(2),
                    ..Default::default()
                },
                children,
            )
        }
        Block::Image {
            src,
            alt,
            width_pct,
        } => (
            "image",
            truncate(alt, 96),
            BlockMetrics {
                image_src: Some(src.clone()),
                width_pct: *width_pct,
                ..Default::default()
            },
            Vec::new(),
        ),
        Block::Footnotes(items) => (
            "footnotes",
            format!("{} item(s)", items.len()),
            BlockMetrics {
                items: Some(items.len()),
                ..Default::default()
            },
            Vec::new(),
        ),
    };

    BlockPlan {
        index,
        path,
        kind: kind.to_string(),
        summary,
        estimated_weight: round3(estimated_weight),
        metrics,
        children,
    }
}

fn ir_block(block: &Block) -> IrBlock {
    match block {
        Block::Paragraph(runs) => IrBlock {
            kind: "paragraph".to_string(),
            text: Some(runs_text(runs)),
            ..IrBlock::empty()
        },
        Block::Heading { level, runs } => IrBlock {
            kind: "heading".to_string(),
            text: Some(runs_text(runs)),
            level: Some(*level),
            ..IrBlock::empty()
        },
        Block::List(items) => IrBlock {
            kind: "list".to_string(),
            items: items
                .iter()
                .map(|item| IrListItem {
                    text: runs_text(&item.runs),
                    level: item.level,
                    ordered: item.ordered,
                })
                .collect(),
            ..IrBlock::empty()
        },
        Block::CodeBlock {
            lang,
            title,
            lines,
            line_numbers,
            start_line,
            columns,
            include_error,
        } => IrBlock {
            kind: "code".to_string(),
            lang: lang.clone(),
            title: title.clone(),
            lines: lines.clone(),
            line_numbers: Some(*line_numbers),
            start_line: Some(*start_line),
            columns: columns.map(code_columns_name).map(ToOwned::to_owned),
            include_error: include_error.clone(),
            ..IrBlock::empty()
        },
        Block::Quote(paras) | Block::Callout { body: paras, .. } => IrBlock {
            kind: "quote".to_string(),
            rows: paras
                .iter()
                .map(|runs| vec![vec![runs_text(runs)]])
                .collect(),
            ..IrBlock::empty()
        },
        Block::Table { headers, rows, .. } => IrBlock {
            kind: "table".to_string(),
            headers: vec![headers.iter().map(|cell| runs_text(cell)).collect()],
            rows: rows
                .iter()
                .map(|row| row.iter().map(|cell| vec![runs_text(cell)]).collect())
                .collect(),
            ..IrBlock::empty()
        },
        Block::ColumnBreak => IrBlock {
            kind: "column_break".to_string(),
            ..IrBlock::empty()
        },
        Block::Columns { left, right } => IrBlock {
            kind: "columns".to_string(),
            left: left.iter().map(ir_block).collect(),
            right: right.iter().map(ir_block).collect(),
            ..IrBlock::empty()
        },
        Block::Image {
            src,
            alt,
            width_pct,
        } => IrBlock {
            kind: "image".to_string(),
            src: Some(src.clone()),
            alt: Some(alt.clone()),
            width_pct: *width_pct,
            ..IrBlock::empty()
        },
        Block::Footnotes(items) => IrBlock {
            kind: "footnotes".to_string(),
            items: items
                .iter()
                .map(|item| IrListItem {
                    text: runs_text(&item.runs),
                    level: item.level,
                    ordered: item.ordered,
                })
                .collect(),
            ..IrBlock::empty()
        },
    }
}

impl IrBlock {
    fn empty() -> Self {
        IrBlock {
            kind: String::new(),
            text: None,
            level: None,
            items: Vec::new(),
            lang: None,
            title: None,
            lines: Vec::new(),
            line_numbers: None,
            start_line: None,
            columns: None,
            include_error: None,
            headers: Vec::new(),
            rows: Vec::new(),
            left: Vec::new(),
            right: Vec::new(),
            src: None,
            alt: None,
            width_pct: None,
        }
    }
}

fn slide_kind_name(kind: &SlideKind) -> &'static str {
    match kind {
        SlideKind::Title { .. } => "title",
        SlideKind::Section => "section",
        SlideKind::Content => "content",
    }
}

fn break_mode_name(mode: BreakMode) -> &'static str {
    match mode {
        BreakMode::Off => "off",
        BreakMode::Simple => "simple",
        BreakMode::Smart => "smart",
    }
}

fn table_fit_name(mode: TableFit) -> &'static str {
    match mode {
        TableFit::Auto => "auto",
        TableFit::Split => "split",
        TableFit::Transpose => "transpose",
        TableFit::Off => "off",
    }
}

fn code_columns_name(mode: CodeColumns) -> &'static str {
    match mode {
        CodeColumns::Single => "single",
        CodeColumns::Auto => "auto",
        CodeColumns::TwoUp => "two-up",
    }
}

fn emu_to_pt(emu: u32) -> f32 {
    round3(emu as f32 / EMU_PER_PT)
}

fn round3(value: f32) -> f32 {
    let rounded = (value * 1000.0).round() / 1000.0;
    if rounded.abs() < 0.0005 {
        0.0
    } else {
        rounded
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (idx, ch) in text.chars().enumerate() {
        if idx >= max_chars {
            out.push_str("...");
            return out;
        }
        out.push(ch);
    }
    out
}
