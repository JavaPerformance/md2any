//! In-memory markdown → multi-format conversion.
//!
//! This is the shared entry point for the CLI (optional), embedding, and the
//! WebAssembly studio. Callers supply markdown plus an optional virtual asset
//! map; no filesystem or network is required when images are either absent,
//! `data:` URIs, or registered in the asset map.

use crate::document::{DocumentOptions, DocumentStyle};
use crate::docx;
use crate::font::PdfFontOptions;
use crate::front_matter;
use crate::html;
use crate::image;
use crate::ir::{Slide, SlideKind};
use crate::layout::Layout;
use crate::math::{MathMode, MathSvgOptions};
use crate::odp;
use crate::odt;
use crate::paginate::{self, BreakMode, PaginationOptions, TableFit};
use crate::parser::{self, ParseOptions};
use crate::pdf::{self, NotesLayout, NotesPageSize};
use crate::pptx;
use crate::theme::Theme;
use crate::toc;
use anyhow::{bail, Context, Result};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

/// Output formats supported by the pure convert API (single-file outputs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Pptx,
    Odp,
    Pdf,
    Docx,
    Odt,
    Html,
}

impl OutputFormat {
    pub fn from_name(name: &str) -> Result<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "pptx" | "powerpoint" => Ok(Self::Pptx),
            "odp" | "impress" => Ok(Self::Odp),
            "pdf" => Ok(Self::Pdf),
            "docx" | "word" => Ok(Self::Docx),
            "odt" | "writer" => Ok(Self::Odt),
            "html" | "htm" | "web" => Ok(Self::Html),
            other => bail!(
                "unsupported format '{other}' (try pptx, odp, pdf, docx, odt, html)"
            ),
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Pptx => "pptx",
            Self::Odp => "odp",
            Self::Pdf => "pdf",
            Self::Docx => "docx",
            Self::Odt => "odt",
            Self::Html => "html",
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            Self::Pptx => {
                "application/vnd.openxmlformats-officedocument.presentationml.presentation"
            }
            Self::Odp => "application/vnd.oasis.opendocument.presentation",
            Self::Pdf => "application/pdf",
            Self::Docx => {
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            }
            Self::Odt => "application/vnd.oasis.opendocument.text",
            Self::Html => "text/html; charset=utf-8",
        }
    }

    pub fn name(self) -> &'static str {
        self.extension()
    }
}

/// Options for [`convert`].
#[derive(Debug, Clone)]
pub struct ConvertOptions {
    pub format: OutputFormat,
    pub theme: Option<String>,
    pub aspect: Option<String>,
    pub layout: Option<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    /// When true and format is HTML, emit the continuous-scroll editor preview
    /// (all slides stacked, `data-line` attributes for caret sync).
    pub editor_preview: bool,
    pub math_mode: MathMode,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            format: OutputFormat::Html,
            theme: None,
            aspect: None,
            layout: None,
            title: None,
            author: None,
            editor_preview: false,
            math_mode: MathMode::Unicode,
        }
    }
}

/// Successful conversion result.
#[derive(Debug, Clone)]
pub struct ConvertResult {
    pub bytes: Vec<u8>,
    pub filename: String,
    pub content_type: String,
    pub slide_count: usize,
    pub title: String,
}

/// One slide for outline / rail UI.
#[derive(Debug, Clone)]
pub struct OutlineSlide {
    pub index: usize,
    pub title: String,
    pub kind: String,
    /// 1-based source line in the markdown (0 if unknown).
    pub source_line: u32,
    /// Whether the slide has speaker notes (`<!-- notes: … -->`).
    pub has_notes: bool,
    /// Speaker notes text when present (studio notes panel / AI).
    pub notes: Option<String>,
}

/// Convert markdown to a single-file office/web artifact entirely in memory.
///
/// `assets` maps markdown image references (e.g. `"photo.png"`, `"assets/x.jpg"`)
/// to raw image bytes. Keys are matched exactly as written in the markdown, and
/// also by file name. When empty, only `data:` URIs and (on native builds with
/// `remote-images`) network URLs work for images.
pub fn convert(
    markdown: &str,
    opts: &ConvertOptions,
    assets: &HashMap<String, Vec<u8>>,
) -> Result<ConvertResult> {
    image::with_virtual_assets(assets, || convert_inner(markdown, opts))
}

/// Parse + paginate and return a slide outline (no full render).
pub fn outline(
    markdown: &str,
    opts: &ConvertOptions,
    assets: &HashMap<String, Vec<u8>>,
) -> Result<Vec<OutlineSlide>> {
    image::with_virtual_assets(assets, || {
        with_cached_deck(markdown, opts, assets, |deck| {
            Ok(deck
                .slides
                .iter()
                .enumerate()
                .map(|(i, s)| OutlineSlide {
                    index: i + 1,
                    title: if s.title.is_empty() {
                        format!("Slide {}", i + 1)
                    } else {
                        s.title.clone()
                    },
                    kind: kind_name(&s.kind).to_string(),
                    source_line: s.source_line,
                    has_notes: s.notes.as_ref().is_some_and(|n| !n.trim().is_empty()),
                    notes: s.notes.clone(),
                })
                .collect())
        })
    })
}

/// One lint finding for studio / CI consumers.
#[derive(Debug, Clone)]
pub struct LintHit {
    pub slide: usize,
    pub kind: String,
    pub detail: String,
}

/// Parse + paginate then run the same checks as CLI `--check`.
pub fn lint(
    markdown: &str,
    opts: &ConvertOptions,
    assets: &HashMap<String, Vec<u8>>,
) -> Result<Vec<LintHit>> {
    image::with_virtual_assets(assets, || {
        with_cached_deck(markdown, opts, assets, |deck| {
            Ok(crate::lint::check(&deck.slides, &deck.theme)
                .into_iter()
                .map(|w| LintHit {
                    slide: w.slide,
                    kind: w.kind.to_string(),
                    detail: w.detail,
                })
                .collect())
        })
    })
}

/// Built-in theme names for UI pickers.
pub fn theme_names() -> &'static [&'static str] {
    &crate::theme::THEME_NAMES
}

/// Built-in layout names for UI pickers.
pub fn layout_names() -> &'static [&'static str] {
    &["clean", "studio", "frame", "bold"]
}

/// Metadata for every slide, plus HTML only for a requested window.
///
/// Used by the browser studio so a 300-page deck does not require generating
/// or mounting every slide on each keystroke.
#[derive(Debug, Clone)]
pub struct PreviewWindow {
    pub title: String,
    pub slide_count: usize,
    /// `class` value for `<body>` (includes `edit`, theme, layout).
    pub body_class: String,
    /// Full CSS for the preview iframe (theme + editor stacking rules).
    pub css: String,
    /// Changes when slide count / breaks / kinds / theme chrome change.
    pub structure_key: String,
    /// One entry per slide (outline); `html` is set only inside the window.
    pub slides: Vec<PreviewSlide>,
    /// 0-based inclusive start of the HTML window.
    pub html_from: usize,
    /// 0-based exclusive end of the HTML window.
    pub html_to: usize,
}

#[derive(Debug, Clone)]
pub struct PreviewSlide {
    pub index: usize,
    pub title: String,
    pub kind: String,
    /// 0-based body line (matches `data-line` on the HTML section).
    pub source_line: u32,
    /// Full `<section class="slide">…</section>` when inside the HTML window.
    pub html: Option<String>,
    /// Stable-ish content fingerprint for incremental DOM patching.
    pub content_key: String,
    /// Whether the slide has speaker notes.
    pub has_notes: bool,
    /// Speaker notes text (always included — small; helps notes UI without a second call).
    pub notes: Option<String>,
}

/// One slide as an export-geometry image (SVG or PNG) for studio ghost / filmstrip.
#[derive(Debug, Clone)]
pub struct SlideImage {
    pub index: usize,
    pub title: String,
    pub format: String,
    pub bytes: Vec<u8>,
}

/// Parse + paginate the whole deck, but only **render HTML** for
/// `html_from..html_to` (0-based, end exclusive, clamped).
///
/// Outline metadata is always returned for every slide so the rail and
/// caret mapping stay global without O(n) DOM.
pub fn preview_window(
    markdown: &str,
    opts: &ConvertOptions,
    assets: &HashMap<String, Vec<u8>>,
    html_from: usize,
    html_to: usize,
) -> Result<PreviewWindow> {
    image::with_virtual_assets(assets, || {
        with_cached_deck(markdown, opts, assets, |deck| {
            let base_dir = Path::new(".");
            let n = deck.slides.len();
            let from = html_from.min(n);
            let to = html_to.min(n).max(from);
            let rtl = deck
                .direction
                .as_deref()
                .map(|s| s.eq_ignore_ascii_case("rtl"))
                .unwrap_or(false);

            let css = html::editor_preview_css(&deck.theme, &deck.layout, rtl);
            let body_class = format!(
                "layout-{} theme-{}{} edit",
                deck.layout.name(),
                deck.theme.name,
                if rtl { " dir-rtl" } else { "" },
            );

            let mut structure = String::with_capacity(n * 24);
            use std::fmt::Write as _;
            let _ = write!(
                structure,
                "{}|{}|{}",
                deck.theme.name,
                deck.layout.name(),
                body_class
            );
            for (i, s) in deck.slides.iter().enumerate() {
                let _ = write!(
                    structure,
                    ";{}:{}:{}",
                    i,
                    s.source_line,
                    kind_name(&s.kind)
                );
            }

            // Fragment HTML cache: same deck + same slide index → skip re-render
            // when scrolling the virtual window (big win on large decks).
            let deck_fp = deck_fingerprint(markdown, opts, assets);
            let mut slides = Vec::with_capacity(n);
            for (i, s) in deck.slides.iter().enumerate() {
                let title = if s.title.is_empty() {
                    format!("Slide {}", i + 1)
                } else {
                    s.title.clone()
                };
                let kind = kind_name(&s.kind).to_string();
                let (html, content_key) = if i >= from && i < to {
                    if let Some((key, frag)) = FRAG_CACHE.with(|c| {
                        c.borrow()
                            .as_ref()
                            .filter(|fc| fc.deck_fp == deck_fp)
                            .and_then(|fc| fc.frags.get(&i).cloned())
                    }) {
                        (Some(frag), key)
                    } else {
                        let frag = html::render_slide_fragment(
                            s,
                            i,
                            n,
                            &deck.theme,
                            &deck.layout,
                            &deck.deck_title,
                            base_dir,
                            None,
                        )?;
                        let key = format!("{:x}", fnv1a64(frag.as_bytes()));
                        FRAG_CACHE.with(|c| {
                            let mut slot = c.borrow_mut();
                            if slot.as_ref().map(|f| f.deck_fp) != Some(deck_fp) {
                                *slot = Some(FragCache {
                                    deck_fp,
                                    frags: HashMap::new(),
                                });
                            }
                            if let Some(fc) = slot.as_mut() {
                                fc.frags.insert(i, (key.clone(), frag.clone()));
                            }
                        });
                        (Some(frag), key)
                    }
                } else {
                    let key = format!("{}:{}:{}", s.source_line, kind, title.len());
                    (None, key)
                };
                slides.push(PreviewSlide {
                    index: i + 1,
                    title,
                    kind,
                    source_line: s.source_line,
                    html,
                    content_key,
                    has_notes: s.notes.as_ref().is_some_and(|n| !n.trim().is_empty()),
                    notes: s.notes.clone(),
                });
            }

            Ok(PreviewWindow {
                title: deck.deck_title.clone(),
                slide_count: n,
                body_class,
                css,
                structure_key: format!("{:x}", fnv1a64(structure.as_bytes())),
                slides,
                html_from: from,
                html_to: to,
            })
        })
    })
}

/// Render a window of slides as SVG (or PNG) using the same geometry as CLI
/// `--format svg|png`. Used by the studio export ghost and filmstrip thumbs.
///
/// `from..to` is 0-based, end exclusive. Format is `"svg"` (default) or `"png"`.
pub fn slide_images(
    markdown: &str,
    opts: &ConvertOptions,
    assets: &HashMap<String, Vec<u8>>,
    from: usize,
    to: usize,
    format: &str,
) -> Result<Vec<SlideImage>> {
    image::with_virtual_assets(assets, || {
        with_cached_deck(markdown, opts, assets, |deck| {
            let n = deck.slides.len();
            let from = from.min(n);
            let to = to.min(n).max(from);
            if from >= to {
                return Ok(Vec::new());
            }
            let base_dir = Path::new(".");
            let want_png = matches!(format.trim().to_ascii_lowercase().as_str(), "png" | "image");
            let slice = &deck.slides[from..to];
            let files = crate::svg::write_files(
                slice,
                &deck.theme,
                &deck.layout,
                &deck.deck_title,
                &deck.author,
                base_dir,
                None,
                deck.direction.as_deref(),
                if want_png {
                    crate::svg::ImageFormat::Png
                } else {
                    crate::svg::ImageFormat::Svg
                },
            )?;
            let fmt_name = if want_png { "png" } else { "svg" };
            Ok(files
                .into_iter()
                .enumerate()
                .map(|(j, f)| {
                    let i = from + j;
                    let title = deck
                        .slides
                        .get(i)
                        .map(|s| {
                            if s.title.is_empty() {
                                format!("Slide {}", i + 1)
                            } else {
                                s.title.clone()
                            }
                        })
                        .unwrap_or_else(|| format!("Slide {}", i + 1));
                    SlideImage {
                        index: i + 1,
                        title,
                        format: fmt_name.to_string(),
                        bytes: f.bytes,
                    }
                })
                .collect())
        })
    })
}

// ---------------------------------------------------------------------------
// Studio IR cache — reuse parse/paginate across window moves & lint/outline
// ---------------------------------------------------------------------------

struct DeckCache {
    fp: u64,
    deck: Deck,
}

struct FragCache {
    deck_fp: u64,
    /// 0-based slide index → (content_key, html fragment)
    frags: HashMap<usize, (String, String)>,
}

thread_local! {
    static DECK_CACHE: RefCell<Option<DeckCache>> = const { RefCell::new(None) };
    static FRAG_CACHE: RefCell<Option<FragCache>> = const { RefCell::new(None) };
}

fn deck_fingerprint(
    markdown: &str,
    opts: &ConvertOptions,
    assets: &HashMap<String, Vec<u8>>,
) -> u64 {
    let mut h = fnv1a64(markdown.as_bytes());
    // Mix option fields that affect parse/paginate/theme.
    for part in [
        opts.theme.as_deref().unwrap_or(""),
        opts.aspect.as_deref().unwrap_or(""),
        opts.layout.as_deref().unwrap_or(""),
        opts.title.as_deref().unwrap_or(""),
        opts.author.as_deref().unwrap_or(""),
        if opts.editor_preview { "1" } else { "0" },
        match opts.math_mode {
            MathMode::Unicode => "u",
            MathMode::Source => "s",
            MathMode::Svg => "v",
        },
    ] {
        h = fnv1a64_continue(h, part.as_bytes());
        h = fnv1a64_continue(h, b"|");
    }
    // Asset map identity: sorted keys + lengths (not full bytes).
    let mut keys: Vec<_> = assets.keys().collect();
    keys.sort();
    for k in keys {
        h = fnv1a64_continue(h, k.as_bytes());
        h = fnv1a64_continue(h, b":");
        let len = assets.get(k).map(|b| b.len()).unwrap_or(0);
        h = fnv1a64_continue(h, &len.to_le_bytes());
        h = fnv1a64_continue(h, b";");
    }
    h
}

fn fnv1a64_continue(mut h: u64, data: &[u8]) -> u64 {
    const PRIME: u64 = 0x100000001b3;
    for &b in data {
        h ^= u64::from(b);
        h = h.wrapping_mul(PRIME);
    }
    h
}

fn with_cached_deck<R>(
    markdown: &str,
    opts: &ConvertOptions,
    assets: &HashMap<String, Vec<u8>>,
    f: impl FnOnce(&Deck) -> Result<R>,
) -> Result<R> {
    let fp = deck_fingerprint(markdown, opts, assets);
    let deck = DECK_CACHE.with(|c| {
        if let Some(cache) = c.borrow().as_ref() {
            if cache.fp == fp {
                return Ok(cache.deck.clone());
            }
        }
        let deck = build_deck(markdown, opts)?;
        *c.borrow_mut() = Some(DeckCache {
            fp,
            deck: deck.clone(),
        });
        Ok::<_, anyhow::Error>(deck)
    })?;
    f(&deck)
}

fn fnv1a64(data: &[u8]) -> u64 {
    const OFF: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let mut h = OFF;
    for &b in data {
        h ^= u64::from(b);
        h = h.wrapping_mul(PRIME);
    }
    h
}

#[derive(Clone)]
struct Deck {
    slides: Vec<Slide>,
    theme: Theme,
    layout: Layout,
    deck_title: String,
    author: String,
    direction: Option<String>,
    transition: Option<String>,
    transition_dur: f32,
}

fn convert_inner(markdown: &str, opts: &ConvertOptions) -> Result<ConvertResult> {
    // Full export: always fresh build (don't reuse editor_preview-cached deck
    // from a different format/options shape). Invalidate so subsequent
    // previews don't serve a stale clone after a convert-only code path.
    let deck = build_deck(markdown, opts)?;
    DECK_CACHE.with(|c| *c.borrow_mut() = None);
    FRAG_CACHE.with(|c| *c.borrow_mut() = None);
    let base_dir = Path::new(".");
    let n = deck.slides.len();
    let title = deck.deck_title.clone();
    let stem = sanitize_filename(&title);

    let bytes = match opts.format {
        OutputFormat::Pptx => pptx::write(
            &deck.slides,
            &deck.theme,
            &deck.layout,
            &deck.deck_title,
            &deck.author,
            base_dir,
            None,
            deck.transition.as_deref(),
            deck.transition_dur,
            deck.direction.as_deref(),
        )?,
        OutputFormat::Odp => odp::write(
            &deck.slides,
            &deck.theme,
            &deck.layout,
            &deck.deck_title,
            &deck.author,
            base_dir,
            None,
            deck.transition.as_deref(),
            deck.transition_dur,
            deck.direction.as_deref(),
        )?,
        OutputFormat::Pdf => {
            // Prefer a system math face (STIX, …) when present so full-page
            // ```math``` slides pack like the showcase PDF without requiring
            // an explicit --pdf-font on every invocation.
            let math_font = crate::font::find_system_math_font();
            let font_options = PdfFontOptions {
                pdf_font: math_font.as_deref(),
                ..Default::default()
            };
            pdf::write_with_font_options(
                &deck.slides,
                &deck.theme,
                &deck.layout,
                &deck.deck_title,
                &deck.author,
                base_dir,
                None,
                None,
                deck.transition.as_deref(),
                deck.transition_dur,
                deck.direction.as_deref(),
                false,
                NotesPageSize::Slide,
                NotesLayout::Auto,
                font_options,
            )?
        }
        OutputFormat::Docx => docx::write_with_options(
            &deck.slides,
            &deck.theme,
            &deck.deck_title,
            &deck.author,
            base_dir,
            None,
            deck.direction.as_deref(),
            &DocumentOptions::new(DocumentStyle::Report),
        )?,
        OutputFormat::Odt => odt::write_with_options(
            &deck.slides,
            &deck.theme,
            &deck.deck_title,
            &deck.author,
            base_dir,
            None,
            deck.direction.as_deref(),
            &DocumentOptions::new(DocumentStyle::Report),
        )?,
        OutputFormat::Html => {
            if opts.editor_preview {
                html::write_opts(
                    &deck.slides,
                    &deck.theme,
                    &deck.layout,
                    &deck.deck_title,
                    &deck.author,
                    base_dir,
                    None,
                    deck.direction.as_deref(),
                    true,
                )?
            } else {
                html::write(
                    &deck.slides,
                    &deck.theme,
                    &deck.layout,
                    &deck.deck_title,
                    &deck.author,
                    base_dir,
                    None,
                    deck.direction.as_deref(),
                )?
            }
        }
    };

    let ext = opts.format.extension();
    Ok(ConvertResult {
        bytes,
        filename: format!("{stem}.{ext}"),
        content_type: opts.format.content_type().to_string(),
        slide_count: n,
        title,
    })
}

fn build_deck(markdown: &str, opts: &ConvertOptions) -> Result<Deck> {
    let (front, body) = front_matter::extract(markdown);

    // Document front-matter wins over ConvertOptions for theme/layout/aspect.
    // Options act as defaults when the markdown omits a key — so the WASM
    // studio (and any host that mirrors toolbar state into options) still
    // reflects `theme: pastel` edits in the source, while a bare document
    // without front-matter keeps using the toolbar defaults.
    let theme_name = non_empty(front.theme.as_deref())
        .map(|s| s.to_string())
        .or_else(|| non_empty(opts.theme.as_deref()).map(|s| s.to_string()))
        .unwrap_or_else(|| "light".into());
    let aspect = non_empty(front.aspect.as_deref())
        .map(|s| s.to_string())
        .or_else(|| non_empty(opts.aspect.as_deref()).map(|s| s.to_string()))
        .unwrap_or_else(|| "16:9".into());
    let font_pref = front.font.clone();
    let mut theme = Theme::resolve(&theme_name, &aspect, font_pref.as_deref())
        .with_context(|| "resolve theme")?;
    if let Some(ov) = &front.style {
        theme
            .apply_override(ov)
            .with_context(|| "apply inline style")?;
    }

    let layout_name = non_empty(front.layout.as_deref())
        .map(|s| s.to_string())
        .or_else(|| non_empty(opts.layout.as_deref()).map(|s| s.to_string()))
        .unwrap_or_else(|| "clean".into());
    let mut layout = Layout::resolve(&layout_name).with_context(|| "resolve layout")?;
    if let Some(lo) = front.style.as_ref().and_then(|ov| ov.layout.as_ref()) {
        layout.apply_override(lo);
    }

    let deck_title = opts
        .title
        .clone()
        .or_else(|| front.title.clone())
        .unwrap_or_else(|| "Presentation".into());
    let author = opts
        .author
        .clone()
        .or_else(|| front.author.clone())
        .unwrap_or_else(|| "md2any".into());

    let pagination = PaginationOptions {
        break_mode: BreakMode::Smart,
        fill: 0.9,
        table_fit: TableFit::Auto,
        ..Default::default()
    };

    // Front-matter wins for math mode/options (same as CLI), so `math: svg`
    // in rich-math.md works in the studio without a CLI flag.
    let math_mode = front
        .math
        .as_deref()
        .and_then(|m| parse_math_mode(m))
        .unwrap_or(opts.math_mode);
    let math_svg = math_svg_from_front(&front);
    let math_macros: Vec<(String, String)> = front
        .math_macros
        .as_ref()
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();

    let mut parsed = parser::parse_with_options(
        &body,
        &front,
        &deck_title,
        ParseOptions {
            math_mode,
            math_svg,
            math_macros,
            include_base_dir: None,
        },
    );

    // Diagram shell-outs are a no-op on wasm / without tools.
    let cache = crate::diagram::cache_dir_for("web");
    let _ = crate::diagram::pre_render(&mut parsed, &cache);

    let slides = paginate::paginate_for_layout_with_options(parsed, &theme, &layout, pagination);
    let slides = if front.toc { toc::inject(slides) } else { slides };

    Ok(Deck {
        slides,
        theme,
        layout,
        deck_title,
        author,
        direction: front.direction.clone(),
        transition: front.transition.clone(),
        transition_dur: front.transition_duration.unwrap_or(0.4),
    })
}

fn kind_name(kind: &SlideKind) -> &'static str {
    match kind {
        SlideKind::Title { .. } => "title",
        SlideKind::Section => "section",
        SlideKind::Content => "content",
    }
}

fn non_empty(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|s| !s.is_empty())
}

fn parse_math_mode(value: &str) -> Option<MathMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "unicode" => Some(MathMode::Unicode),
        "source" | "raw" | "off" => Some(MathMode::Source),
        "svg" | "image" | "images" => Some(MathMode::Svg),
        _ => None,
    }
}

fn math_svg_from_front(front: &crate::ir::FrontMatter) -> MathSvgOptions {
    let scale = front.math_scale.unwrap_or(1.0).clamp(0.35, 3.0);
    let block_align = front
        .math_block_align
        .as_deref()
        .map(|a| match a.trim().to_ascii_lowercase().as_str() {
            "left" | "start" => crate::math::MathBlockAlign::Left,
            "right" | "end" => crate::math::MathBlockAlign::Right,
            _ => crate::math::MathBlockAlign::Center,
        })
        .unwrap_or(crate::math::MathBlockAlign::Center);
    let max_height_px = front
        .math_max_height
        .map(|h| h.clamp(24.0, 1200.0).round() as u16);
    MathSvgOptions {
        scale_percent: (scale * 100.0).round() as u16,
        max_height_px,
        block_align,
    }
}

fn sanitize_filename(title: &str) -> String {
    let s: String = title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else if c.is_whitespace() {
                '-'
            } else {
                '_'
            }
        })
        .collect();
    let s = s.trim_matches('-').trim_matches('_');
    if s.is_empty() {
        "document".into()
    } else {
        s.chars().take(64).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"---
title: Test Deck
theme: light
---

# Hello

A short slide.

---

## Two

- one
- two
"#;

    #[test]
    fn convert_html_and_pdf() {
        let assets = HashMap::new();
        let html = convert(
            SAMPLE,
            &ConvertOptions {
                format: OutputFormat::Html,
                editor_preview: true,
                ..Default::default()
            },
            &assets,
        )
        .expect("html");
        assert!(html.bytes.starts_with(b"<!doctype html>") || html.bytes.starts_with(b"<!DOCTYPE"));
        assert!(html.slide_count >= 2);

        let pdf = convert(
            SAMPLE,
            &ConvertOptions {
                format: OutputFormat::Pdf,
                ..Default::default()
            },
            &assets,
        )
        .expect("pdf");
        assert!(pdf.bytes.starts_with(b"%PDF"));
        assert!(pdf.filename.ends_with(".pdf"));
    }

    #[test]
    fn convert_pptx() {
        let assets = HashMap::new();
        let r = convert(
            SAMPLE,
            &ConvertOptions {
                format: OutputFormat::Pptx,
                ..Default::default()
            },
            &assets,
        )
        .expect("pptx");
        // OOXML zip magic
        assert_eq!(&r.bytes[..2], b"PK");
    }

    #[test]
    fn outline_lists_slides() {
        let assets = HashMap::new();
        let o = outline(SAMPLE, &ConvertOptions::default(), &assets).expect("outline");
        assert!(o.len() >= 2);
        assert!(!o[0].has_notes);
    }

    #[test]
    fn outline_exposes_speaker_notes() {
        let md = r#"---
title: Notes
---

# Hello

<!-- notes: say hi -->

## Next
"#;
        let assets = HashMap::new();
        let o = outline(md, &ConvertOptions::default(), &assets).expect("outline");
        let with_notes = o.iter().find(|s| s.has_notes);
        assert!(
            with_notes.is_some(),
            "expected a slide with notes, got: {:?}",
            o.iter()
                .map(|s| (s.index, s.title.clone(), s.has_notes))
                .collect::<Vec<_>>()
        );
        assert!(
            with_notes
                .unwrap()
                .notes
                .as_deref()
                .unwrap_or("")
                .contains("say hi"),
            "notes text: {:?}",
            with_notes.unwrap().notes
        );
    }

    #[test]
    fn slide_images_svg_window() {
        let assets = HashMap::new();
        let imgs = slide_images(
            SAMPLE,
            &ConvertOptions {
                format: OutputFormat::Html,
                editor_preview: true,
                ..Default::default()
            },
            &assets,
            0,
            1,
            "svg",
        )
        .expect("svg");
        assert_eq!(imgs.len(), 1);
        assert_eq!(imgs[0].format, "svg");
        assert!(imgs[0].bytes.starts_with(b"<svg") || imgs[0].bytes.starts_with(b"<?xml"));
    }

    /// Display math expands into multiple output lines; `source_line` must still
    /// match the original body line so the studio / editor halo tracks the caret.
    #[test]
    fn source_lines_survive_display_math() {
        let md = r#"---
title: T
---

# One

intro

$$e^{i\pi} + 1 = 0$$

---

## Two

body
"#;
        let assets = HashMap::new();
        let o = outline(md, &ConvertOptions::default(), &assets).expect("outline");
        let (_f, body) = crate::front_matter::extract(md);
        let two_line = body
            .lines()
            .enumerate()
            .find(|(_, l)| l.starts_with("## Two"))
            .map(|(i, _)| i as u32)
            .expect("## Two in body");
        let two = o.iter().find(|s| s.title == "Two").expect("slide Two");
        assert_eq!(
            two.source_line, two_line,
            "slide Two source_line should be original body line {two_line}, got {}",
            two.source_line
        );

        // Caret on `## Two` must resolve to that slide (not the previous one).
        let mut pick = &o[0];
        for s in &o {
            if s.source_line <= two_line {
                pick = s;
            } else {
                break;
            }
        }
        assert_eq!(pick.title, "Two");
    }

    #[test]
    fn front_matter_theme_wins_over_options() {
        let md = r#"---
title: T
theme: pastel
layout: bold
aspect: 4:3
---

# Hi
"#;
        let assets = HashMap::new();
        // Studio always passes toolbar defaults; the document must still win.
        let r = convert(
            md,
            &ConvertOptions {
                format: OutputFormat::Html,
                editor_preview: true,
                theme: Some("midnight".into()),
                layout: Some("clean".into()),
                aspect: Some("16:9".into()),
                ..Default::default()
            },
            &assets,
        )
        .expect("convert");
        let s = String::from_utf8_lossy(&r.bytes);
        assert!(
            s.contains("theme-pastel"),
            "front-matter theme should win, html head: {}",
            s.lines().take(20).collect::<Vec<_>>().join("\n")
        );
        assert!(
            s.contains("layout-bold"),
            "front-matter layout should win"
        );
    }

    #[test]
    fn options_fill_in_when_front_matter_omits_theme() {
        let md = "# Hi\n\nNo front matter here.\n";
        let assets = HashMap::new();
        let r = convert(
            md,
            &ConvertOptions {
                format: OutputFormat::Html,
                editor_preview: true,
                theme: Some("terminal".into()),
                ..Default::default()
            },
            &assets,
        )
        .expect("convert");
        let s = String::from_utf8_lossy(&r.bytes);
        assert!(
            s.contains("theme-terminal"),
            "options should apply when FM omits theme"
        );
    }

    #[test]
    fn front_matter_math_svg_emits_math_images() {
        let md = r#"---
title: M
math: svg
math_max_height: 180
---

# Hi

$$
\frac{a}{b}
$$
"#;
        let assets = HashMap::new();
        let r = convert(
            md,
            &ConvertOptions {
                format: OutputFormat::Html,
                editor_preview: true,
                // Studio default — front-matter must still win.
                math_mode: MathMode::Unicode,
                ..Default::default()
            },
            &assets,
        )
        .expect("convert");
        let s = String::from_utf8_lossy(&r.bytes);
        assert!(
            s.contains("data:image/svg+xml") || s.contains("math-image"),
            "math: svg front-matter should produce SVG math images, got: {}",
            &s[s.find("slide-content").unwrap_or(0)..][..s.len().min(400)]
        );
        assert!(
            !s.contains("$$"),
            "display math delimiters should not survive in HTML output"
        );
    }

    #[test]
    fn preview_window_only_renders_requested_range() {
        let mut md = String::from("---\ntitle: Big\ntheme: light\n---\n\n");
        for i in 1..=20 {
            md.push_str(&format!("## Slide {i}\n\nbody {i}\n\n---\n\n"));
        }
        let assets = HashMap::new();
        let win = preview_window(
            &md,
            &ConvertOptions {
                format: OutputFormat::Html,
                editor_preview: true,
                ..Default::default()
            },
            &assets,
            5,
            8,
        )
        .expect("preview_window");
        assert!(win.slide_count >= 20, "expected many slides");
        assert_eq!(win.html_from, 5);
        assert_eq!(win.html_to, 8);
        let with_html: Vec<_> = win
            .slides
            .iter()
            .enumerate()
            .filter(|(_, s)| s.html.is_some())
            .map(|(i, _)| i)
            .collect();
        assert_eq!(with_html, vec![5, 6, 7]);
        // Outside the window: metadata only.
        assert!(win.slides[0].html.is_none());
        assert!(win.slides[5].html.as_ref().unwrap().contains("slide"));
        assert!(!win.structure_key.is_empty());
    }
}
