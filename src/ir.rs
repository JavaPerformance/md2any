//! Intermediate representation for the deck.
//!
//! Every renderer (PPTX, ODP, PDF, DOCX, ODT, HTML) walks a tree of [`Slide`]s,
//! each of which contains a list of [`Block`]s. The parser builds this
//! tree from markdown; the paginator may rewrite it (splitting long slides
//! into `(cont.)` chunks); the renderers only ever read it.
//!
//! Keeping the IR small and renderer-agnostic is the load-bearing design
//! choice in md2any: adding another output format means writing one new
//! `fn write(slides, theme, ...)` file, nothing else.

use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeColumns {
    Single,
    Auto,
    TwoUp,
}

/// Document-level options parsed from the YAML block at the top of the
/// markdown file. Every field is optional and may be overridden by a CLI
/// flag; missing fields fall back to compile-time defaults.
#[derive(Debug, Default, Deserialize, Clone)]
pub struct FrontMatter {
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub author: Option<String>,
    pub date: Option<String>,
    pub theme: Option<String>,
    pub aspect: Option<String>,
    pub font: Option<String>,
    pub layout: Option<String>,
    /// Math preprocessor mode: "unicode" (default), "source", or "svg".
    pub math: Option<String>,
    /// Exact math macro substitutions applied inside math spans before the
    /// built-in translator/renderer sees them.
    pub math_macros: Option<BTreeMap<String, String>>,
    /// Scale factor for generated SVG display math, e.g. 0.85 or 1.25.
    #[serde(alias = "math-scale")]
    pub math_scale: Option<f32>,
    /// Alignment for generated SVG display math: "left", "center", or "right".
    #[serde(alias = "math-block-align")]
    pub math_block_align: Option<String>,
    /// Maximum generated SVG display-math height in logical pixels.
    #[serde(alias = "math-max-height")]
    pub math_max_height: Option<f32>,
    /// Flowing document profile for DOCX/ODT: "plain", "report",
    /// "handout", or "speaker-notes".
    pub doc_style: Option<String>,
    /// Slide pagination strategy: "smart" (default), "simple", or "off".
    pub break_mode: Option<String>,
    /// Percentage of the estimated slide content area to fill before breaking.
    pub break_fill: Option<f32>,
    /// Wide-table handling: "auto" (default), "split", "transpose", or "off".
    pub table_fit: Option<String>,
    /// Code block palette: "dark" (default), "light", or "match".
    #[serde(alias = "code-theme")]
    pub code_theme: Option<String>,
    /// Code block column flow: "single" (default), "auto", or "two-up".
    #[serde(alias = "code-columns")]
    pub code_columns: Option<String>,
    #[serde(default)]
    pub toc: bool,
    pub logo: Option<String>,
    /// Slide-to-slide transition: "none" (default), "fade", "push", "wipe", "cover".
    pub transition: Option<String>,
    /// Transition duration in seconds (default 0.4).
    pub transition_duration: Option<f32>,
    /// Text direction: "ltr" (default) or "rtl" for Arabic/Hebrew layout.
    /// Affects paragraph direction + alignment in PPTX/ODP. PDF glyph shaping
    /// of right-to-left scripts requires an embedded font (not supplied).
    pub direction: Option<String>,
    /// Inline theme overrides (colours, fonts, sizes, title alignment, layout
    /// geometry) equivalent to a `--theme-file`, layered over the named
    /// `theme`. The `--serve --edit` style panel reads and writes this block,
    /// but it can also be authored by hand.
    pub style: Option<crate::theme::ThemeOverride>,
}

/// A contiguous span of text with uniform styling. The parser splits
/// rich-text into runs whenever the active formatting flags change (entering
/// or leaving bold / italic / code / strikethrough / a link), so a styled
/// paragraph becomes a list of runs that the renderers concatenate.
#[derive(Debug, Clone, Default)]
pub struct Run {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strike: bool,
    /// If set, this run is rendered as an underlined, accent-coloured
    /// hyperlink. The URL is registered with the format's relationship
    /// table at render time.
    pub link: Option<String>,
}

impl Run {
    pub fn plain(s: impl Into<String>) -> Self {
        Run {
            text: s.into(),
            ..Default::default()
        }
    }
}

/// Per-column text alignment from a GFM table delimiter row (`:---`, `:---:`,
/// `---:`). pulldown-cmark's "no alignment" maps to `Left` (the default).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnAlign {
    Left,
    Center,
    Right,
}

/// One entry inside a [`Block::List`]. Nesting is encoded with `level` (0 = top)
/// rather than a recursive `Vec<ListItem>` because that mirrors how
/// pulldown-cmark emits list events and keeps the IR cheap to pattern-match
/// across renderers.
#[derive(Debug, Clone)]
pub struct ListItem {
    pub runs: Vec<Run>,
    pub level: u8,
    pub ordered: bool,
}

impl ListItem {
    /// True when this item is a GFM task-list entry (`- [ ]` / `- [x]`). The
    /// parser renders the checkbox as a leading `☐`/`☑` run, which stands in
    /// for the list bullet — so renderers should suppress their normal bullet
    /// glyph and not draw both.
    pub fn is_task(&self) -> bool {
        self.runs
            .first()
            .map(|r| r.text.starts_with('☐') || r.text.starts_with('☑'))
            .unwrap_or(false)
    }
}

/// A single block-level element on a slide. The renderer's job is to take a
/// flat sequence of these and place them inside the slide area. Layout-aware
/// helpers in each writer (height estimation, column packing) consume the
/// same enum.
#[derive(Debug, Clone)]
pub enum Block {
    /// A run of flowing text. Multiple paragraphs become multiple `Paragraph`s.
    Paragraph(Vec<Run>),
    /// H3+ headings inside a slide (H1/H2 already produce slides). `level`
    /// is 3..=6.
    Heading { level: u8, runs: Vec<Run> },
    /// Bulleted or numbered list. Items can be at different `level`s for
    /// nesting; `ordered` is per-item so mixed lists are possible.
    List(Vec<ListItem>),
    /// Fenced code block, optionally with a language hint and filename
    /// caption. `line_numbers` defaults to true when `lines.len() > 5`;
    /// `start_line` preserves numbering across paginated continuation chunks.
    CodeBlock {
        lang: Option<String>,
        title: Option<String>,
        lines: Vec<String>,
        line_numbers: bool,
        start_line: usize,
        /// Optional per-fence column flow override from `columns=...`.
        columns: Option<CodeColumns>,
        /// Set when a `file=...#Lx-Ly` include could not be loaded.
        /// Renderers still receive fallback `lines`; `--check` uses this to
        /// make the failed include visible in CI.
        include_error: Option<String>,
    },
    /// Block quote — each inner `Vec<Run>` is one paragraph of the quote.
    Quote(Vec<Vec<Run>>),
    /// GitHub-style callout / admonition (`> [!NOTE]` etc.). `kind` is one of
    /// `note`, `tip`, `important`, `warning`, `caution`; `body` holds the
    /// paragraphs after the marker. Renderers that don't style it specially
    /// fall back to rendering `body` as a plain quote.
    Callout { kind: String, body: Vec<Vec<Run>> },
    /// GitHub-flavoured table. `headers` is the first row; `rows` are the
    /// data rows. Cells are run lists so inline formatting works inside them.
    /// `aligns` carries the per-column alignment from the GFM delimiter row
    /// (`:---`, `:---:`, `---:`); it has one entry per column (may be shorter
    /// than the widest row — renderers default missing entries to `Left`).
    Table {
        headers: Vec<Vec<Run>>,
        rows: Vec<Vec<Vec<Run>>>,
        aligns: Vec<ColumnAlign>,
    },
    /// Sentinel emitted by the `:::` marker. Consumed by [`crate::paginate`]
    /// when it folds the block list into a `Columns` block; renderers should
    /// never see one.
    ColumnBreak,
    /// Side-by-side columns produced by `:::`. Each side is a fresh block
    /// list that renders independently within half the slide width.
    Columns { left: Vec<Block>, right: Vec<Block> },
    /// Local-path image. The renderer is responsible for resolving `src`
    /// relative to the deck file and embedding the bytes appropriately.
    Image {
        src: String,
        alt: String,
        /// Optional Pandoc-style `{width=N%}` attribute. When set, slide
        /// renderers clamp the image to this fraction of the slide content
        /// area (1..=100). Flowing-document renderers (DOCX / ODT) ignore
        /// it; for those, the consumer app handles sizing.
        width_pct: Option<u8>,
    },
    /// Inline footnote definitions associated with this slide. Rendered as
    /// small muted text at the bottom of the slide.
    Footnotes(Vec<ListItem>),
}

/// Slides come in three flavours which the layouts treat very differently.
#[derive(Debug, Clone)]
pub enum SlideKind {
    /// Deck opener — centered hero title with optional subtitle / author / date.
    Title {
        subtitle: Option<String>,
        author: Option<String>,
        date: Option<String>,
    },
    /// Section divider — emitted for `# H1` after the title slide. Usually
    /// styled as a full-bleed coloured page with the section name centered.
    Section,
    /// Regular content — title bar at the top, blocks below.
    Content,
}

/// One slide ready for rendering. Outputs that flow rather than paginate
/// (DOCX / ODT) walk the same `Vec<Slide>` and concatenate the contents.
#[derive(Debug, Clone)]
pub struct Slide {
    pub kind: SlideKind,
    pub title: String,
    pub blocks: Vec<Block>,
    /// 0-based line in the markdown *body* (front-matter excluded) where this
    /// slide begins — the heading/rule that opened it, or 0 for the title and
    /// auto-injected slides. Continuation slides inherit their parent's line.
    /// Used by the `--serve --edit` editor to focus the slide under the caret.
    pub source_line: u32,
    /// Speaker notes from `<!-- notes: -->` HTML comments in the source.
    /// Rendered in PPTX/ODP only — PDF/DOCX/ODT drop them.
    pub notes: Option<String>,
    /// Per-slide full-bleed background image from `<!-- bg: path -->`.
    /// Propagates through continuation slides created by pagination.
    pub bg_image: Option<String>,
    /// Per-slide layout hint from `<!-- layout: NAME -->`. Most hints are
    /// handled by a small post-parse pass that rearranges the block list
    /// (e.g. splits an image off into its own column for `image-left` /
    /// `image-right`). Renderer-owned hints such as `image-full` remain here.
    pub layout_hint: Option<String>,
}

impl Slide {
    /// The layout-hint *kind* — the first whitespace token of `layout_hint`,
    /// e.g. `image-left` from `image-left valign=center`. Lets a hint carry
    /// extra options (like `valign=`) without breaking kind matching.
    pub fn layout_kind(&self) -> Option<&str> {
        self.layout_hint
            .as_deref()
            .and_then(|h| h.split_whitespace().next())
    }

    /// Vertical alignment of column content from a `valign=` option on the
    /// layout hint (`top` default, `center`, or `bottom`). Applies to the
    /// `image-left`/`image-right`/`:::` two-column layouts.
    pub fn valign(&self) -> &str {
        self.layout_hint
            .as_deref()
            .and_then(|h| h.split_whitespace().find_map(|t| t.strip_prefix("valign=")))
            .map(|v| match v {
                "center" | "middle" => "center",
                "bottom" | "end" => "bottom",
                _ => "top",
            })
            .unwrap_or("top")
    }

    /// The left-column fraction (0..1) for an `image-left`/`image-right` slide
    /// whose hint carries `width=NN%`. `width=` sizes the *image* column, so for
    /// `image-right` (image on the right) the left/text column gets `1 - frac`.
    /// `None` means the default even split.
    pub fn col_frac(&self) -> Option<f32> {
        let hint = self.layout_hint.as_deref()?;
        let w = hint
            .split_whitespace()
            .find_map(|t| t.strip_prefix("width="))?;
        let pct: f32 = w.trim_end_matches('%').parse().ok()?;
        let frac = (pct / 100.0).clamp(0.15, 0.85);
        Some(if self.layout_kind() == Some("image-right") {
            1.0 - frac
        } else {
            frac
        })
    }

    /// Horizontal text alignment for the slide's content from an `align=`
    /// option (`left` default, `center`, or `right`), set by `<!-- align: … -->`
    /// or appended to a layout hint. Applies to the slide's text blocks.
    pub fn text_align(&self) -> &str {
        self.layout_hint
            .as_deref()
            .and_then(|h| h.split_whitespace().find_map(|t| t.strip_prefix("align=")))
            .map(|v| match v {
                "center" | "centre" | "middle" => "center",
                "right" | "end" => "right",
                _ => "left",
            })
            .unwrap_or("left")
    }

    /// Return the sole image on a slide that requested full-page image layout.
    pub fn full_page_image(&self) -> Option<(&str, &str, Option<u8>)> {
        if self.layout_kind() != Some("image-full") {
            return None;
        }

        let mut image = None;
        for block in &self.blocks {
            match block {
                Block::Image {
                    src,
                    alt,
                    width_pct,
                } => {
                    if image.is_some() {
                        return None;
                    }
                    image = Some((src.as_str(), alt.as_str(), *width_pct));
                }
                Block::Footnotes(_) => {}
                _ => return None,
            }
        }
        image
    }

    /// Return the sole code block on a slide that requested full-page text layout.
    pub fn full_page_code(&self) -> Option<(&[String], Option<&str>)> {
        if self.layout_kind() != Some("text-full") {
            return None;
        }

        let mut code = None;
        for block in &self.blocks {
            match block {
                Block::CodeBlock { lang, lines, .. } => {
                    if code.is_some() {
                        return None;
                    }
                    code = Some((lines.as_slice(), lang.as_deref()));
                }
                Block::Footnotes(_) => {}
                _ => return None,
            }
        }
        code
    }
}

/// Concatenate all run text into a single plain string. Strips all
/// formatting; useful for height estimation and lint checks where only the
/// character count matters.
pub fn runs_text(runs: &[Run]) -> String {
    let mut s = String::new();
    for r in runs {
        s.push_str(&r.text);
    }
    s
}
