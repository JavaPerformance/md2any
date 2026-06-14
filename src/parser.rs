//! Markdown → slide IR parser.
//!
//! Uses [pulldown-cmark](https://docs.rs/pulldown-cmark) for the event stream
//! and walks the events with a stateful builder (the private `State` struct
//! in this module) that emits
//! [`Slide`]s as it encounters H1/H2 headings, `---` rules, and the like.
//!
//! Two preprocessing passes run before pulldown-cmark sees the source:
//!   1. [`crate::math::translate_with_options`] converts `$...$` / `$$...$$`
//!      LaTeX-ish spans into Unicode/source/SVG math output.
//!   2. A local pass converts the `:::` column-break sentinel into an HTML
//!      comment that pulldown-cmark passes through verbatim — we then
//!      collapse it back into a [`Block::Columns`] later.
//!
//! Footnote definitions are captured into a side table and materialised
//! onto whatever slide referenced them, so notes appear at the bottom of
//! the slide where they're used.

use crate::ir::*;
use crate::math::{MathMode, MathOptions, MathSvgOptions};
use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ParseOptions {
    pub math_mode: MathMode,
    pub math_macros: Vec<(String, String)>,
    pub math_svg: MathSvgOptions,
    pub include_base_dir: Option<PathBuf>,
}

impl Default for ParseOptions {
    fn default() -> Self {
        ParseOptions {
            math_mode: MathMode::Unicode,
            math_macros: Vec::new(),
            math_svg: MathSvgOptions::default(),
            include_base_dir: None,
        }
    }
}

/// Parse markdown into a flat sequence of [`Slide`]s.
///
/// `front` supplies the deck's front-matter so the parser can seed the first
/// slide with the user-provided title before any markdown is seen.
/// `fallback_title` is the deck title used when a content slide is reached
/// before any heading exists — usually the input filename's stem.
pub fn parse(input: &str, front: &FrontMatter, fallback_title: &str) -> Vec<Slide> {
    parse_with_options(input, front, fallback_title, ParseOptions::default())
}

pub fn parse_with_options(
    input: &str,
    front: &FrontMatter,
    fallback_title: &str,
    options: ParseOptions,
) -> Vec<Slide> {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_FOOTNOTES);

    let math_options = resolve_math_options(front, &options);
    let preprocessed = preprocess(input, &math_options);
    let parser = Parser::new_ext(&preprocessed, opts);

    let mut st = State::new(front, fallback_title, options.include_base_dir.as_deref());
    for event in parser {
        st.handle(event);
    }
    let mut slides = st.finish();
    apply_layout_hints(&mut slides);
    slides
}

/// Honour `<!-- layout: image-left | image-right -->` by reshaping the
/// slide's block list into a two-column layout: the first Image on one
/// side, everything else on the other. Slides without exactly one image,
/// or with an unrecognised/post-renderer hint, pass through unchanged.
fn apply_layout_hints(slides: &mut Vec<Slide>) {
    for slide in slides {
        let Some(hint) = slide.layout_hint.as_deref() else {
            continue;
        };
        if hint != "image-left" && hint != "image-right" {
            continue;
        }
        let image_count = slide
            .blocks
            .iter()
            .filter(|b| matches!(b, Block::Image { .. }))
            .count();
        if image_count != 1 {
            continue;
        }
        let mut image_block: Option<Block> = None;
        let mut rest: Vec<Block> = Vec::with_capacity(slide.blocks.len());
        for b in std::mem::take(&mut slide.blocks) {
            if image_block.is_none() && matches!(b, Block::Image { .. }) {
                image_block = Some(b);
            } else {
                rest.push(b);
            }
        }
        let image = match image_block {
            Some(b) => b,
            None => continue,
        };
        let (left, right) = match hint {
            "image-left" => (vec![image], rest),
            _ => (rest, vec![image]),
        };
        slide.blocks = vec![Block::Columns { left, right }];
    }
}

fn resolve_math_options(front: &FrontMatter, options: &ParseOptions) -> MathOptions {
    let mut macros = front
        .math_macros
        .as_ref()
        .map(|m| {
            m.iter()
                .map(|(from, to)| (from.clone(), to.clone()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    macros.extend(options.math_macros.clone());
    MathOptions {
        mode: options.math_mode,
        macros,
        svg: options.math_svg,
    }
}

fn preprocess(input: &str, math_options: &MathOptions) -> String {
    // First pass: math preprocessing (skips fenced code blocks itself).
    let math_translated = crate::math::translate_with_options(input, math_options);
    let mut out = String::with_capacity(math_translated.len());
    let mut in_code = false;
    for line in math_translated.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_code = !in_code;
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if !in_code && line.trim() == ":::" {
            out.push_str("\n<!--md2any-col-->\n\n");
            continue;
        }
        if !in_code {
            // Pandoc-style image attribute: `![alt](src){width=NN%}`.
            // Strip the `{...}` from the source (pulldown-cmark would
            // emit it as literal text) and replace with a custom HTML
            // comment that the parser dispatches to the preceding image.
            out.push_str(&extract_image_attrs(line));
            out.push('\n');
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Walks `line` looking for `]({...){width=NN%}` patterns immediately
/// after an image syntax. Returns the rewritten line; if no attribute is
/// found, returns the input unchanged.
///
/// `)` and `{` and `}` are all ASCII so byte-level scanning is safe for
/// the search, but the rest of `line` may contain multi-byte UTF-8 — we
/// must splice the input by byte offsets (not push bytes as chars) so
/// Unicode characters survive intact.
fn extract_image_attrs(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut out = String::with_capacity(line.len());
    let mut copied = 0; // next unwritten byte offset
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b')' && bytes.get(i + 1) == Some(&b'{') {
            if let Some(close_rel) = bytes[i + 2..].iter().position(|&b| b == b'}') {
                let attr = &line[i + 2..i + 2 + close_rel];
                if let Some(pct) = parse_width_pct(attr) {
                    // Flush everything before and including the `)`, then
                    // emit the comment and skip past the closing `}`.
                    out.push_str(&line[copied..=i]);
                    out.push_str(&format!("<!--md2any-imgwidth:{}-->", pct));
                    i = i + 2 + close_rel + 1;
                    copied = i;
                    continue;
                }
            }
        }
        i += 1;
    }
    out.push_str(&line[copied..]);
    out
}

/// Expand tab characters in a code line to spaces at 4-column tab stops.
fn expand_code_tabs(line: &str) -> String {
    const TAB: usize = 4;
    let mut out = String::with_capacity(line.len());
    let mut col = 0usize;
    for ch in line.chars() {
        if ch == '\t' {
            let n = TAB - (col % TAB);
            out.extend(std::iter::repeat(' ').take(n));
            col += n;
        } else {
            out.push(ch);
            col += 1;
        }
    }
    out
}

fn parse_width_pct(attr: &str) -> Option<u8> {
    let attr = attr.trim();
    let rest = attr.strip_prefix("width=")?.trim();
    let n = rest.strip_suffix('%')?.trim().parse::<u32>().ok()?;
    // Clamp out-of-range percentages rather than rejecting them: a rejected
    // attribute leaks its literal `{width=…}` text into the slide body (and
    // forces a spurious continuation slide). Downstream renderers clamp too.
    Some(n.clamp(1, 100) as u8)
}

struct State<'a> {
    front: &'a FrontMatter,
    fallback_title: &'a str,
    include_base_dir: Option<&'a Path>,
    slides: Vec<Slide>,
    current: Slide,
    started_real_content: bool,
    first_h1_consumed: bool,

    runs: Vec<Run>,
    bold: u32,
    italic: u32,
    strike: u32,
    link: Option<String>,

    heading_capture: Option<u8>,

    list_stack: Vec<bool>,
    list_items: Vec<ListItem>,
    item_runs: Vec<Run>,
    in_item: bool,

    in_code: bool,
    code_lang: Option<String>,
    code_title: Option<String>,
    code_include: Option<CodeInclude>,
    code_start_line: usize,
    code_columns: Option<CodeColumns>,
    code_buf: String,
    /// Accumulates an inline `<svg>…</svg>` block (which can arrive across
    /// several HTML events) until its closing tag, at which point it becomes a
    /// rasterised image instead of being dropped.
    svg_buf: Option<String>,

    in_blockquote: u32,
    quote_paragraphs: Vec<Vec<Run>>,

    in_table: bool,
    in_thead: bool,
    in_cell: bool,
    table_headers: Vec<Vec<Run>>,
    table_rows: Vec<Vec<Vec<Run>>>,
    table_row: Vec<Vec<Run>>,
    table_aligns: Vec<crate::ir::ColumnAlign>,
    cell_runs: Vec<Run>,

    in_image: bool,
    image_src: String,
    image_alt: String,

    /// Footnote label → numeric index assigned on first reference.
    footnote_numbers: std::collections::HashMap<String, u32>,
    /// Footnote label → captured definition runs.
    footnote_defs: std::collections::HashMap<String, Vec<Run>>,
    /// Label currently being captured into footnote_defs, if any.
    capturing_footnote: Option<String>,
    /// For each slide (by index in `slides`), the ordered list of footnote
    /// labels referenced on that slide. After parsing finishes we materialize
    /// a footnotes block on each referencing slide.
    slide_footnote_refs: Vec<Vec<String>>,
    /// References captured on the current (still-open) slide.
    current_footnote_refs: Vec<String>,
}

impl<'a> State<'a> {
    fn new(
        front: &'a FrontMatter,
        fallback_title: &'a str,
        include_base_dir: Option<&'a Path>,
    ) -> Self {
        let initial = if front.title.is_some() {
            Slide {
                kind: SlideKind::Title {
                    subtitle: front.subtitle.clone(),
                    author: front.author.clone(),
                    date: front.date.clone(),
                },
                title: front.title.clone().unwrap_or_else(|| fallback_title.into()),
                blocks: Vec::new(),
                notes: None,
                bg_image: None,
                layout_hint: None,
            }
        } else {
            Slide {
                kind: SlideKind::Content,
                title: String::new(),
                blocks: Vec::new(),
                notes: None,
                bg_image: None,
                layout_hint: None,
            }
        };
        State {
            front,
            fallback_title,
            include_base_dir,
            slides: Vec::new(),
            current: initial,
            started_real_content: front.title.is_some(),
            first_h1_consumed: false,
            runs: Vec::new(),
            bold: 0,
            italic: 0,
            strike: 0,
            link: None,
            heading_capture: None,
            list_stack: Vec::new(),
            list_items: Vec::new(),
            item_runs: Vec::new(),
            in_item: false,
            in_code: false,
            code_lang: None,
            code_title: None,
            code_include: None,
            code_start_line: 1,
            code_columns: None,
            code_buf: String::new(),
            svg_buf: None,
            in_blockquote: 0,
            quote_paragraphs: Vec::new(),
            in_table: false,
            in_thead: false,
            in_cell: false,
            table_headers: Vec::new(),
            table_rows: Vec::new(),
            table_row: Vec::new(),
            table_aligns: Vec::new(),
            cell_runs: Vec::new(),
            in_image: false,
            image_src: String::new(),
            image_alt: String::new(),
            footnote_numbers: std::collections::HashMap::new(),
            footnote_defs: std::collections::HashMap::new(),
            capturing_footnote: None,
            slide_footnote_refs: Vec::new(),
            current_footnote_refs: Vec::new(),
        }
    }

    fn current_attrs(&self) -> Run {
        Run {
            text: String::new(),
            bold: self.bold > 0,
            italic: self.italic > 0,
            strike: self.strike > 0,
            code: false,
            link: self.link.clone(),
        }
    }

    fn push_text(&mut self, text: &str, is_code: bool) {
        if self.in_image {
            self.image_alt.push_str(text);
            return;
        }
        let mut run = self.current_attrs();
        run.text = text.to_string();
        run.code = is_code;

        let sink: &mut Vec<Run> = if self.in_cell {
            &mut self.cell_runs
        } else if self.in_item {
            &mut self.item_runs
        } else {
            &mut self.runs
        };

        if let Some(last) = sink.last_mut() {
            if last.bold == run.bold
                && last.italic == run.italic
                && last.strike == run.strike
                && last.code == run.code
                && last.link == run.link
            {
                last.text.push_str(&run.text);
                return;
            }
        }
        sink.push(run);
    }

    fn flush_paragraph(&mut self) {
        if self.runs.is_empty() {
            return;
        }
        // If we're inside a footnote definition, paragraph runs belong to the
        // footnote text — they're picked up when the definition tag closes.
        if self.capturing_footnote.is_some() {
            return;
        }
        let runs = std::mem::take(&mut self.runs);
        if self.in_blockquote > 0 {
            self.quote_paragraphs.push(runs);
        } else {
            self.current.blocks.push(Block::Paragraph(runs));
            self.started_real_content = true;
        }
    }

    fn open_slide(&mut self, kind: SlideKind, title: String) {
        let needs_flush = !self.current.title.is_empty()
            || !self.current.blocks.is_empty()
            || self.started_real_content;
        if needs_flush {
            self.slide_footnote_refs
                .push(std::mem::take(&mut self.current_footnote_refs));
            self.slides.push(std::mem::replace(
                &mut self.current,
                Slide {
                    kind: kind.clone(),
                    title,
                    blocks: Vec::new(),
                    notes: None,
                    bg_image: None,
                    layout_hint: None,
                },
            ));
        } else {
            self.current.kind = kind;
            self.current.title = title;
        }
        self.started_real_content = true;
    }

    /// Turn an accumulated inline `<svg>…</svg>` into a rasterisable image
    /// block (base64 SVG data URI), routed through the normal image pipeline.
    fn push_inline_svg(&mut self, svg: &str) {
        let Some(start) = svg.find("<svg") else {
            return;
        };
        let end = svg
            .find("</svg>")
            .map(|i| i + "</svg>".len())
            .unwrap_or(svg.len());
        let markup = &svg[start..end];
        self.flush_paragraph();
        let uri = format!(
            "data:image/svg+xml;base64,{}",
            crate::math::base64(markup.as_bytes())
        );
        self.current.blocks.push(Block::Image {
            src: uri,
            alt: String::new(),
            width_pct: None,
        });
        self.started_real_content = true;
    }

    fn handle(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start_tag(tag),
            Event::End(tag) => self.end_tag(tag),
            Event::Text(t) => {
                if self.in_code {
                    self.code_buf.push_str(&t);
                } else if self.heading_capture.is_some() {
                    self.push_text(&t, false);
                } else {
                    self.push_text(&t, false);
                }
            }
            Event::Code(c) => {
                self.push_text(&c, true);
            }
            Event::Html(c) | Event::InlineHtml(c) => {
                // Inline `<svg>…</svg>` arrives as one or more HTML events;
                // accumulate it and rasterise on the closing tag rather than
                // dropping it (markdown HTML is otherwise ignored).
                if self.svg_buf.is_some() || c.contains("<svg") {
                    let buf = self.svg_buf.get_or_insert_with(String::new);
                    buf.push_str(&c);
                    if buf.contains("</svg>") {
                        let svg = self.svg_buf.take().unwrap_or_default();
                        self.push_inline_svg(&svg);
                    }
                    return;
                }
                let s = c.trim();
                if s == "<!--md2any-col-->" {
                    self.flush_paragraph();
                    self.current.blocks.push(Block::ColumnBreak);
                    self.started_real_content = true;
                } else if let Some(path) = extract_bg(s) {
                    self.current.bg_image = Some(path);
                    self.started_real_content = true;
                } else if let Some(note) = extract_note(s) {
                    let existing = self.current.notes.take().unwrap_or_default();
                    let combined = if existing.is_empty() {
                        note
                    } else {
                        format!("{existing}\n\n{note}")
                    };
                    self.current.notes = Some(combined);
                } else if let Some(name) = extract_layout(s) {
                    self.current.layout_hint = Some(name);
                    self.started_real_content = true;
                } else if let Some(pct) = extract_img_width(s) {
                    // Attach to the most recent Image block on the
                    // current slide. If no image precedes the directive
                    // (e.g. typo or reordering), drop it silently.
                    for b in self.current.blocks.iter_mut().rev() {
                        if let Block::Image { width_pct, .. } = b {
                            *width_pct = Some(pct);
                            break;
                        }
                    }
                }
            }
            Event::FootnoteReference(label) => {
                let label_s = label.into_string();
                let next_idx = (self.footnote_numbers.len() as u32) + 1;
                let n = *self
                    .footnote_numbers
                    .entry(label_s.clone())
                    .or_insert(next_idx);
                if !self.current_footnote_refs.contains(&label_s) {
                    self.current_footnote_refs.push(label_s);
                }
                self.push_text(&superscript(n), false);
            }
            Event::SoftBreak => {
                self.push_text(" ", false);
            }
            Event::HardBreak => {
                self.push_text(" ", false);
            }
            Event::Rule => {
                self.flush_paragraph();
                let title = if self.current.title.is_empty() {
                    self.fallback_title.to_string()
                } else {
                    self.current.title.clone()
                };
                self.open_slide(SlideKind::Content, title);
            }
            Event::TaskListMarker(checked) => {
                let mark = if checked { "☑ " } else { "☐ " };
                self.push_text(mark, false);
            }
        }
    }

    fn start_tag(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => {}
            Tag::Heading { level, .. } => {
                self.flush_paragraph();
                let lvl = heading_to_u8(level);
                self.heading_capture = Some(lvl);
                self.runs.clear();
            }
            Tag::BlockQuote => {
                self.flush_paragraph();
                // Only reset the accumulator when entering the OUTERMOST quote;
                // a nested `>>`/`>>>` must keep the outer levels' paragraphs
                // (resetting here dropped everything but the deepest level).
                if self.in_blockquote == 0 {
                    self.quote_paragraphs = Vec::new();
                }
                self.in_blockquote += 1;
            }
            Tag::CodeBlock(kind) => {
                self.flush_paragraph();
                self.in_code = true;
                let info = match kind {
                    CodeBlockKind::Fenced(info) => parse_fence_info(&info),
                    _ => FenceInfo::default(),
                };
                self.code_lang = info.lang;
                self.code_title = info.title;
                self.code_include = info.include;
                self.code_start_line = info.start_line;
                self.code_columns = info.columns;
                self.code_buf.clear();
            }
            Tag::List(start) => {
                self.flush_paragraph();
                if self.in_item && !self.item_runs.is_empty() {
                    let runs = std::mem::take(&mut self.item_runs);
                    let level = (self.list_stack.len() as u8).saturating_sub(1);
                    let ordered = self.list_stack.last().copied().unwrap_or(false);
                    self.list_items.push(ListItem {
                        runs,
                        level,
                        ordered,
                    });
                }
                self.list_stack.push(start.is_some());
            }
            Tag::Item => {
                self.in_item = true;
                self.item_runs.clear();
            }
            Tag::Emphasis => self.italic += 1,
            Tag::Strong => self.bold += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Link { dest_url, .. } => {
                self.link = Some(dest_url.to_string());
            }
            Tag::Image { dest_url, .. } => {
                self.flush_paragraph();
                self.in_image = true;
                self.image_src = dest_url.to_string();
                self.image_alt.clear();
            }
            Tag::Table(aligns) => {
                self.flush_paragraph();
                self.in_table = true;
                self.table_headers.clear();
                self.table_rows.clear();
                self.table_aligns = aligns
                    .iter()
                    .map(|a| match a {
                        Alignment::Center => crate::ir::ColumnAlign::Center,
                        Alignment::Right => crate::ir::ColumnAlign::Right,
                        // None and Left both render left-aligned.
                        _ => crate::ir::ColumnAlign::Left,
                    })
                    .collect();
            }
            Tag::FootnoteDefinition(label) => {
                self.flush_paragraph();
                let label_s = label.into_string();
                // Reserve a number if the definition appears before any
                // reference (rare but possible).
                let next_idx = (self.footnote_numbers.len() as u32) + 1;
                self.footnote_numbers
                    .entry(label_s.clone())
                    .or_insert(next_idx);
                self.capturing_footnote = Some(label_s);
                self.runs.clear();
            }
            Tag::TableHead => {
                self.in_thead = true;
            }
            Tag::TableRow => {
                self.table_row.clear();
            }
            Tag::TableCell => {
                self.in_cell = true;
                self.cell_runs.clear();
            }
            _ => {}
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.flush_paragraph();
            }
            TagEnd::Heading(level) => {
                let lvl = self.heading_capture.take().unwrap_or(heading_to_u8(level));
                let runs = std::mem::take(&mut self.runs);
                let title = runs_text(&runs);
                if lvl == 1 {
                    if !self.first_h1_consumed && self.front.title.is_none() {
                        self.first_h1_consumed = true;
                        let subtitle = subtitle_from_runs(&runs);
                        self.open_slide(
                            SlideKind::Title {
                                subtitle,
                                author: self.front.author.clone(),
                                date: self.front.date.clone(),
                            },
                            title,
                        );
                    } else {
                        self.first_h1_consumed = true;
                        self.open_slide(SlideKind::Section, title);
                    }
                } else if lvl == 2 {
                    self.open_slide(SlideKind::Content, title);
                } else {
                    self.current
                        .blocks
                        .push(Block::Heading { level: lvl, runs });
                    self.started_real_content = true;
                }
            }
            TagEnd::BlockQuote => {
                self.flush_paragraph();
                if self.in_blockquote > 0 {
                    self.in_blockquote -= 1;
                }
                if self.in_blockquote == 0 {
                    let paras = std::mem::take(&mut self.quote_paragraphs);
                    if !paras.is_empty() {
                        self.current.blocks.push(Block::Quote(paras));
                        self.started_real_content = true;
                    }
                }
            }
            TagEnd::CodeBlock => {
                let fallback_code = std::mem::take(&mut self.code_buf);
                let lang = self.code_lang.take();
                let title = self.code_title.take();
                let include = self.code_include.take();
                let columns = self.code_columns.take();
                let mut include_error = None;
                let (code, start_line) = if let Some(include) = include {
                    match load_code_include(self.include_base_dir, &include) {
                        Ok(code) => (code, include.start_line),
                        Err(e) => {
                            let detail = e.to_string();
                            eprintln!(
                                "md2any: warning: code include {} failed: {e}",
                                include.display
                            );
                            include_error = Some(format!("{}: {}", include.display, detail));
                            let body = if fallback_code.trim().is_empty() {
                                format!("md2any include failed: {e}")
                            } else {
                                fallback_code
                            };
                            (body, include.start_line)
                        }
                    }
                } else {
                    (fallback_code, self.code_start_line)
                };
                // Expand tabs to 4-column tab stops: the embedded fonts have no
                // tab glyph (it renders as a notdef box) and renderers advance
                // by glyph width, so a literal `\t` both tofus and breaks
                // indentation. Doing it here fixes every backend at once.
                let lines: Vec<String> = code
                    .trim_end_matches('\n')
                    .split('\n')
                    .map(expand_code_tabs)
                    .collect();
                let line_numbers = lines.len() > 5;
                self.current.blocks.push(Block::CodeBlock {
                    lang,
                    title,
                    lines,
                    line_numbers,
                    start_line,
                    columns,
                    include_error,
                });
                self.started_real_content = true;
                self.in_code = false;
                self.code_start_line = 1;
                self.code_columns = None;
            }
            TagEnd::List(_) => {
                let _ordered = self.list_stack.pop().unwrap_or(false);
                if self.list_stack.is_empty() && !self.list_items.is_empty() {
                    let items = std::mem::take(&mut self.list_items);
                    self.current.blocks.push(Block::List(items));
                    self.started_real_content = true;
                }
            }
            TagEnd::Item => {
                let runs = std::mem::take(&mut self.item_runs);
                let level = (self.list_stack.len() as u8).saturating_sub(1);
                let ordered = self.list_stack.last().copied().unwrap_or(false);
                if !runs.is_empty() {
                    self.list_items.push(ListItem {
                        runs,
                        level,
                        ordered,
                    });
                }
                self.in_item = false;
            }
            TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
            TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            TagEnd::Link => {
                self.link = None;
            }
            TagEnd::Image => {
                if self.in_image {
                    let src = std::mem::take(&mut self.image_src);
                    let alt = std::mem::take(&mut self.image_alt);
                    self.in_image = false;
                    if !src.is_empty() {
                        self.current.blocks.push(Block::Image {
                            src,
                            alt,
                            width_pct: None,
                        });
                        self.started_real_content = true;
                    }
                }
            }
            TagEnd::Table => {
                self.in_table = false;
                let headers = std::mem::take(&mut self.table_headers);
                let rows = std::mem::take(&mut self.table_rows);
                let aligns = std::mem::take(&mut self.table_aligns);
                self.current.blocks.push(Block::Table {
                    headers,
                    rows,
                    aligns,
                });
                self.started_real_content = true;
            }
            TagEnd::TableHead => {
                self.in_thead = false;
                self.table_headers = std::mem::take(&mut self.table_row);
            }
            TagEnd::TableRow => {
                let row = std::mem::take(&mut self.table_row);
                self.table_rows.push(row);
            }
            TagEnd::TableCell => {
                let runs = std::mem::take(&mut self.cell_runs);
                self.table_row.push(runs);
                self.in_cell = false;
            }
            TagEnd::FootnoteDefinition => {
                if let Some(label) = self.capturing_footnote.take() {
                    let runs = std::mem::take(&mut self.runs);
                    if !runs.is_empty() {
                        self.footnote_defs.insert(label, runs);
                    }
                }
            }
            _ => {}
        }
    }

    fn finish(mut self) -> Vec<Slide> {
        self.flush_paragraph();
        if !self.current.title.is_empty()
            || !self.current.blocks.is_empty()
            || self.slides.is_empty()
        {
            if self.current.title.is_empty() {
                self.current.title = self.fallback_title.into();
            }
            self.slide_footnote_refs
                .push(std::mem::take(&mut self.current_footnote_refs));
            self.slides.push(self.current);
        }

        // Materialize a footnotes block on each slide that referenced any.
        for (i, refs) in self.slide_footnote_refs.iter().enumerate() {
            if refs.is_empty() {
                continue;
            }
            let Some(slide) = self.slides.get_mut(i) else {
                continue;
            };
            let mut items: Vec<ListItem> = Vec::new();
            for label in refs {
                let n = *self.footnote_numbers.get(label).unwrap_or(&0);
                let body = self.footnote_defs.get(label).cloned().unwrap_or_default();
                let mut runs: Vec<Run> = Vec::new();
                runs.push(Run::plain(format!("{}. ", n)));
                runs.extend(body);
                items.push(ListItem {
                    runs,
                    level: 0,
                    ordered: false,
                });
            }
            slide.blocks.push(Block::Footnotes(items));
        }

        self.slides
    }
}

fn superscript(n: u32) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        let sup = match ch {
            '0' => '⁰',
            '1' => '¹',
            '2' => '²',
            '3' => '³',
            '4' => '⁴',
            '5' => '⁵',
            '6' => '⁶',
            '7' => '⁷',
            '8' => '⁸',
            '9' => '⁹',
            other => other,
        };
        out.push(sup);
    }
    out
}

fn heading_to_u8(l: HeadingLevel) -> u8 {
    match l {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn extract_bg(s: &str) -> Option<String> {
    let s = s.trim();
    if !s.starts_with("<!--") || !s.ends_with("-->") {
        return None;
    }
    let inner = s[4..s.len() - 3].trim();
    let lower = inner.to_ascii_lowercase();
    let body = if let Some(b) = lower.strip_prefix("bg:") {
        inner[inner.len() - b.len()..].trim()
    } else if let Some(b) = lower.strip_prefix("background:") {
        inner[inner.len() - b.len()..].trim()
    } else {
        return None;
    };
    if body.is_empty() {
        None
    } else {
        Some(body.to_string())
    }
}

fn extract_img_width(s: &str) -> Option<u8> {
    let s = s.trim();
    if !s.starts_with("<!--") || !s.ends_with("-->") {
        return None;
    }
    let inner = s[4..s.len() - 3].trim();
    let rest = inner.strip_prefix("md2any-imgwidth:")?.trim();
    rest.parse::<u8>().ok().filter(|n| (1..=100).contains(n))
}

/// Recognised values: `image-left`, `image-right`, `image-full`, `text-full`.
/// Anything else returns `None` (silently ignored — better than failing the
/// whole render for a typo in a comment).
fn extract_layout(s: &str) -> Option<String> {
    let s = s.trim();
    if !s.starts_with("<!--") || !s.ends_with("-->") {
        return None;
    }
    let inner = s[4..s.len() - 3].trim();
    let lower = inner.to_ascii_lowercase();
    let body = lower.strip_prefix("layout:")?.trim();
    match body {
        "image-left" | "image-right" | "image-full" | "text-full" => Some(body.to_string()),
        _ => None,
    }
}

fn extract_note(s: &str) -> Option<String> {
    let s = s.trim();
    if !s.starts_with("<!--") || !s.ends_with("-->") {
        return None;
    }
    let inner = s[4..s.len() - 3].trim();
    let lower = inner.to_ascii_lowercase();
    let body = if lower.starts_with("speaker notes:") {
        inner["speaker notes:".len()..].trim()
    } else if lower.starts_with("notes:") {
        inner["notes:".len()..].trim()
    } else {
        return None;
    };
    if body.is_empty() {
        None
    } else {
        Some(body.to_string())
    }
}

#[derive(Debug, Clone)]
struct FenceInfo {
    lang: Option<String>,
    title: Option<String>,
    include: Option<CodeInclude>,
    start_line: usize,
    columns: Option<CodeColumns>,
}

impl Default for FenceInfo {
    fn default() -> Self {
        Self {
            lang: None,
            title: None,
            include: None,
            start_line: 1,
            columns: None,
        }
    }
}

#[derive(Debug, Clone)]
struct CodeInclude {
    path: String,
    display: String,
    start_line: usize,
    end_line: Option<usize>,
}

fn parse_fence_info(info: &str) -> FenceInfo {
    let info = info.trim();
    if info.is_empty() {
        return FenceInfo::default();
    }
    let mut out = FenceInfo {
        start_line: 1,
        ..Default::default()
    };
    let mut title_parts = Vec::new();
    for token in split_fence_tokens(info) {
        if let Some(value) = token.strip_prefix("file=") {
            if let Some(include) = parse_code_include(value) {
                out.start_line = include.start_line;
                if out.lang.is_none() {
                    out.lang = lang_from_path(&include.path);
                }
                if out.title.is_none() && title_parts.is_empty() {
                    out.title = Some(include.display.clone());
                }
                out.include = Some(include);
            }
        } else if let Some(value) = token.strip_prefix("title=") {
            out.title = Some(value.to_string());
            title_parts.clear();
        } else if let Some(value) = token.strip_prefix("start=") {
            if let Ok(n) = value.parse::<usize>() {
                out.start_line = n.max(1);
            }
        } else if let Some(value) = token
            .strip_prefix("columns=")
            .or_else(|| token.strip_prefix("cols="))
            .or_else(|| token.strip_prefix("code-columns="))
        {
            out.columns = parse_code_columns(value);
        } else if out.lang.is_none() {
            out.lang = Some(token);
        } else {
            title_parts.push(token);
        }
    }
    if out.title.is_none() && !title_parts.is_empty() {
        out.title = Some(title_parts.join(" "));
    }
    out
}

fn split_fence_tokens(info: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for ch in info.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' && quote.is_some() {
            escaped = true;
            continue;
        }
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }
        if ch == '"' || ch == '\'' {
            quote = Some(ch);
        } else if ch.is_whitespace() {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn parse_code_include(value: &str) -> Option<CodeInclude> {
    let (path, fragment) = value.split_once('#').unwrap_or((value, ""));
    if path.trim().is_empty() {
        return None;
    }
    let (start_line, end_line) = parse_line_fragment(fragment).unwrap_or((1, None));
    Some(CodeInclude {
        path: path.to_string(),
        display: value.to_string(),
        start_line,
        end_line,
    })
}

fn parse_line_fragment(fragment: &str) -> Option<(usize, Option<usize>)> {
    if fragment.trim().is_empty() {
        return None;
    }
    let fragment = fragment.trim().trim_start_matches('L');
    let (start, end) = fragment
        .split_once('-')
        .or_else(|| fragment.split_once(".."))
        .unwrap_or((fragment, ""));
    let start_line = parse_line_number(start)?;
    let end_line = if end.trim().is_empty() {
        None
    } else {
        Some(parse_line_number(end)?)
    };
    Some((start_line.max(1), end_line.map(|n| n.max(start_line))))
}

fn parse_line_number(value: &str) -> Option<usize> {
    value.trim().trim_start_matches('L').parse::<usize>().ok()
}

fn parse_code_columns(value: &str) -> Option<CodeColumns> {
    let normalized = value.trim().to_ascii_lowercase().replace('_', "-");
    match normalized.as_str() {
        "1" | "one" | "single" | "off" | "none" => Some(CodeColumns::Single),
        "auto" | "smart" => Some(CodeColumns::Auto),
        "2" | "two" | "two-up" | "twoup" | "columns" => Some(CodeColumns::TwoUp),
        _ => None,
    }
}

fn load_code_include(base_dir: Option<&Path>, include: &CodeInclude) -> std::io::Result<String> {
    let path = Path::new(&include.path);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else if let Some(base_dir) = base_dir {
        base_dir.join(path)
    } else {
        path.to_path_buf()
    };
    let text = std::fs::read_to_string(&path)?;
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return Ok(String::new());
    }
    let start_idx = include.start_line.saturating_sub(1).min(lines.len());
    let end_idx = include
        .end_line
        .unwrap_or(lines.len())
        .min(lines.len())
        .max(start_idx);
    Ok(lines[start_idx..end_idx].join("\n"))
}

fn lang_from_path(path: &str) -> Option<String> {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let lang = match ext.as_str() {
        "rs" => "rust",
        "py" => "python",
        "js" | "mjs" | "cjs" => "js",
        "ts" | "tsx" => "ts",
        "jsx" => "jsx",
        "go" => "go",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" => "cpp",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "scala" | "sc" => "scala",
        "cs" => "csharp",
        "ps1" | "psm1" | "psd1" => "powershell",
        "hs" | "lhs" => "haskell",
        "bcpl" => "bcpl",
        "vue" => "vue",
        "svelte" => "svelte",
        "astro" => "astro",
        "graphql" | "gql" => "graphql",
        "tf" | "tfvars" | "hcl" => "hcl",
        "dockerfile" | "containerfile" => "dockerfile",
        "rb" => "ruby",
        "sh" | "bash" | "zsh" => "bash",
        "sql" => "sql",
        "diff" | "patch" => "diff",
        "http" => "http",
        "ini" | "cfg" | "conf" | "env" | "properties" | "props" => "properties",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "html" | "htm" => "html",
        "css" => "css",
        "md" | "markdown" => "md",
        _ => return None,
    };
    Some(lang.to_string())
}

fn subtitle_from_runs(runs: &[Run]) -> Option<String> {
    let text = runs_text(runs);
    if let Some((_, after)) = text.split_once(": ") {
        let s = after.trim();
        if !s.is_empty() {
            return Some(s.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_image_attrs_preserves_non_ascii() {
        // Regression: a previous implementation cast each byte to a char,
        // which broke any multi-byte UTF-8 codepoint in body paragraphs
        // (Greek letters, em-dashes, math operator output).
        let input = "Renders as: α — β with Δt₀ = 2L/c";
        assert_eq!(extract_image_attrs(input), input);
    }

    #[test]
    fn extract_image_attrs_rewrites_width_attribute() {
        let out = extract_image_attrs("![alt](path){width=50%} trailing");
        assert!(out.contains("<!--md2any-imgwidth:50-->"));
        assert!(out.contains("trailing"));
        assert!(!out.contains("{width=50%}"));
    }

    #[test]
    fn extract_image_attrs_ignores_unrelated_braces() {
        // `)` followed by `{...}` that isn't a width attribute passes through.
        let input = "function call(x){body}";
        assert_eq!(extract_image_attrs(input), input);
    }

    #[test]
    fn extract_image_attrs_with_attribute_and_unicode_after() {
        // Both behaviours together: width attribute rewritten, trailing
        // Greek letter survives intact.
        let out = extract_image_attrs("![a](b){width=30%} then α");
        assert!(out.contains("<!--md2any-imgwidth:30-->"));
        assert!(out.contains("α"));
    }
}
