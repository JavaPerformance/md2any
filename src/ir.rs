//! Intermediate representation for the deck.
//!
//! Every renderer (PPTX, ODP, PDF, DOCX, ODT) walks a tree of [`Slide`]s,
//! each of which contains a list of [`Block`]s. The parser builds this
//! tree from markdown; the paginator may rewrite it (splitting long slides
//! into `(cont.)` chunks); the renderers only ever read it.
//!
//! Keeping the IR small and renderer-agnostic is the load-bearing design
//! choice in md2any: adding a sixth output format means writing one new
//! `fn write(slides, theme, ...)` file, nothing else.

use serde::Deserialize;

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
    /// caption. `line_numbers` defaults to true when `lines.len() > 5`.
    CodeBlock {
        lang: Option<String>,
        title: Option<String>,
        lines: Vec<String>,
        line_numbers: bool,
    },
    /// Block quote — each inner `Vec<Run>` is one paragraph of the quote.
    Quote(Vec<Vec<Run>>),
    /// GitHub-flavoured table. `headers` is the first row; `rows` are the
    /// data rows. Cells are run lists so inline formatting works inside them.
    Table {
        headers: Vec<Vec<Run>>,
        rows: Vec<Vec<Vec<Run>>>,
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
    /// Speaker notes from `<!-- notes: -->` HTML comments in the source.
    /// Rendered in PPTX/ODP only — PDF/DOCX/ODT drop them.
    pub notes: Option<String>,
    /// Per-slide full-bleed background image from `<!-- bg: path -->`.
    /// Propagates through continuation slides created by pagination.
    pub bg_image: Option<String>,
    /// Per-slide layout hint from `<!-- layout: NAME -->`. Recognised by a
    /// small post-parse pass that rearranges the block list (e.g. splits
    /// an image off into its own column for `image-left` / `image-right`).
    /// Renderers themselves don't read this field — by the time pagination
    /// runs, the transform has already happened.
    pub layout_hint: Option<String>,
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
