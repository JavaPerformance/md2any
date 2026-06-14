//! `md2any` CLI entry point.
//!
//! Parses arguments, resolves theme + layout + aspect, drives the parse →
//! paginate → render → write pipeline, and handles the long tail of
//! convenience flags (`--watch`, `--serve`, `--handout`, `--with-notes`,
//! the `--help-*` self-documenting outputs, etc.). All real work is in
//! the library modules; this file is plumbing.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use std::path::{Path, PathBuf};
use std::time::Instant;

use md2any::{
    diagram, document, docx, front_matter, html, image, layout, lint, odp, odt, paginate, parser,
    pdf, pptx, render_plan, serve, svg, theme, toc,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Pptx,
    Odp,
    Pdf,
    Docx,
    Odt,
    Html,
    Svg,
    Png,
}

impl Format {
    fn from_extension(ext: &str) -> Option<Format> {
        match ext.to_lowercase().as_str() {
            "pptx" => Some(Format::Pptx),
            "odp" => Some(Format::Odp),
            "pdf" => Some(Format::Pdf),
            "docx" => Some(Format::Docx),
            "odt" => Some(Format::Odt),
            "html" | "htm" => Some(Format::Html),
            "svg" => Some(Format::Svg),
            "png" => Some(Format::Png),
            _ => None,
        }
    }
    fn from_name(s: &str) -> anyhow::Result<Format> {
        match s.to_lowercase().as_str() {
            "pptx" | "powerpoint" => Ok(Format::Pptx),
            "odp" | "impress" => Ok(Format::Odp),
            "pdf" => Ok(Format::Pdf),
            "docx" | "word" => Ok(Format::Docx),
            "odt" | "writer" | "opendocument" => Ok(Format::Odt),
            "html" | "web" | "browser" => Ok(Format::Html),
            "svg" => Ok(Format::Svg),
            "png" => Ok(Format::Png),
            other => anyhow::bail!(
                "unknown format: {} (try pptx, odp, pdf, docx, odt, html, svg, or png)",
                other
            ),
        }
    }
    fn ext(&self) -> &'static str {
        match self {
            Format::Pptx => "pptx",
            Format::Odp => "odp",
            Format::Pdf => "pdf",
            Format::Docx => "docx",
            Format::Odt => "odt",
            Format::Html => "html",
            Format::Svg => "svg",
            Format::Png => "png",
        }
    }
    fn name(&self) -> &'static str {
        match self {
            Format::Pptx => "pptx",
            Format::Odp => "odp",
            Format::Pdf => "pdf",
            Format::Docx => "docx",
            Format::Odt => "odt",
            Format::Html => "html",
            Format::Svg => "svg",
            Format::Png => "png",
        }
    }

    fn is_slide_image_sequence(&self) -> bool {
        matches!(self, Format::Svg | Format::Png)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum BreakModeArg {
    Smart,
    Simple,
    Off,
}

impl From<BreakModeArg> for paginate::BreakMode {
    fn from(value: BreakModeArg) -> Self {
        match value {
            BreakModeArg::Smart => paginate::BreakMode::Smart,
            BreakModeArg::Simple => paginate::BreakMode::Simple,
            BreakModeArg::Off => paginate::BreakMode::Off,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum TableFitArg {
    Auto,
    Split,
    Transpose,
    Off,
}

impl From<TableFitArg> for paginate::TableFit {
    fn from(value: TableFitArg) -> Self {
        match value {
            TableFitArg::Auto => paginate::TableFit::Auto,
            TableFitArg::Split => paginate::TableFit::Split,
            TableFitArg::Transpose => paginate::TableFit::Transpose,
            TableFitArg::Off => paginate::TableFit::Off,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum CodeThemeArg {
    Dark,
    Light,
    Match,
}

impl From<CodeThemeArg> for theme::CodeTheme {
    fn from(value: CodeThemeArg) -> Self {
        match value {
            CodeThemeArg::Dark => theme::CodeTheme::Dark,
            CodeThemeArg::Light => theme::CodeTheme::Light,
            CodeThemeArg::Match => theme::CodeTheme::Match,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum CodeColumnsArg {
    Single,
    Auto,
    TwoUp,
}

impl From<CodeColumnsArg> for md2any::ir::CodeColumns {
    fn from(value: CodeColumnsArg) -> Self {
        match value {
            CodeColumnsArg::Single => md2any::ir::CodeColumns::Single,
            CodeColumnsArg::Auto => md2any::ir::CodeColumns::Auto,
            CodeColumnsArg::TwoUp => md2any::ir::CodeColumns::TwoUp,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum MathModeArg {
    Unicode,
    Source,
    Svg,
}

impl From<MathModeArg> for md2any::math::MathMode {
    fn from(value: MathModeArg) -> Self {
        match value {
            MathModeArg::Unicode => md2any::math::MathMode::Unicode,
            MathModeArg::Source => md2any::math::MathMode::Source,
            MathModeArg::Svg => md2any::math::MathMode::Svg,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum MathBlockAlignArg {
    Left,
    Center,
    Right,
}

impl From<MathBlockAlignArg> for md2any::math::MathBlockAlign {
    fn from(value: MathBlockAlignArg) -> Self {
        match value {
            MathBlockAlignArg::Left => md2any::math::MathBlockAlign::Left,
            MathBlockAlignArg::Center => md2any::math::MathBlockAlign::Center,
            MathBlockAlignArg::Right => md2any::math::MathBlockAlign::Right,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum NotesPageSizeArg {
    Slide,
    A4,
}

impl From<NotesPageSizeArg> for pdf::NotesPageSize {
    fn from(value: NotesPageSizeArg) -> Self {
        match value {
            NotesPageSizeArg::Slide => pdf::NotesPageSize::Slide,
            NotesPageSizeArg::A4 => pdf::NotesPageSize::A4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum NotesLayoutArg {
    Auto,
    Below,
    SideBySide,
}

impl From<NotesLayoutArg> for pdf::NotesLayout {
    fn from(value: NotesLayoutArg) -> Self {
        match value {
            NotesLayoutArg::Auto => pdf::NotesLayout::Auto,
            NotesLayoutArg::Below => pdf::NotesLayout::Below,
            NotesLayoutArg::SideBySide => pdf::NotesLayout::SideBySide,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum DocStyleArg {
    Plain,
    Report,
    Handout,
    SpeakerNotes,
}

impl From<DocStyleArg> for document::DocumentStyle {
    fn from(value: DocStyleArg) -> Self {
        match value {
            DocStyleArg::Plain => document::DocumentStyle::Plain,
            DocStyleArg::Report => document::DocumentStyle::Report,
            DocStyleArg::Handout => document::DocumentStyle::Handout,
            DocStyleArg::SpeakerNotes => document::DocumentStyle::SpeakerNotes,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ServeFormatArg {
    Pdf,
    Html,
    Svg,
    Png,
}

impl From<ServeFormatArg> for serve::ServeFormat {
    fn from(value: ServeFormatArg) -> Self {
        match value {
            ServeFormatArg::Pdf => serve::ServeFormat::Pdf,
            ServeFormatArg::Html => serve::ServeFormat::Html,
            ServeFormatArg::Svg => serve::ServeFormat::Svg,
            ServeFormatArg::Png => serve::ServeFormat::Png,
        }
    }
}

#[derive(Parser)]
#[command(
    name = "md2any",
    about = "Fast Markdown → PowerPoint / OpenDocument / PDF converter. Single binary, cross-platform.",
    version,
    long_about = "Converts Markdown into a slide deck or flowing document. Output format auto-detects\n\
                  from the -o extension, or explicit via --format. Defaults to .pptx.\n\n\
                  Eight formats supported: pptx, odp, pdf, docx, odt, html, svg, png.\n  \
                  pptx/odp/pdf/html paginate one slide per page; svg/png write one file per slide.\n  \
                  docx/odt flow continuously with --doc-style plain|report|handout|speaker-notes.\n\n\
                  Pagination rules (slide formats only):\n  \
                  - First # H1 (or front-matter `title`) → title slide\n  \
                  - Subsequent # H1 → section divider slide\n  \
                  - ## H2 → new content slide\n  \
                  - `---` (horizontal rule) → explicit slide break\n  \
                  - `:::` on its own line → side-by-side columns within a slide\n  \
                  - Lists with > 12 items → auto two-column layout\n  \
                  - `<!-- layout: image-left|image-right|image-full|text-full -->` → layout hints\n  \
                  - `![alt](src){width=N%}` → resize the image to N% of column\n  \
                  - Long content auto-paginates into '(cont.)' slides\n  \
                  - `--break-mode smart|simple|off` controls continuation splitting\n  \
                  - `--break-fill PCT` controls how tightly slides are packed\n\n\
                  Code includes: use fenced blocks like ```rust file=src/main.rs#L20-L80\n  \
                  to pull source snippets while preserving real line numbers.\n\n\
                  Wide tables: `--table-fit auto|split|transpose|off` controls\n  \
                  column splitting and compact portrait transposition.\n\n\
                  Code blocks: `--code-theme dark|light|match` controls the\n  \
                  code-block palette independently of the main deck theme. Default: dark.\n\n\
                  Code columns: `--code-columns single|auto|two-up` controls\n  \
                  landscape two-up code flow. Fences can override with `columns=1|auto|2`.\n\n\
                  Math: `--math unicode|source|svg` keeps the default Unicode\n  \
                  translator, preserves source, or renders display math with a built-in SVG box layout.\n  \
                  `--math-scale`, `--math-block-align`, and `--math-max-height` tune generated display math.\n\n\
                  Front-matter (YAML between `---` fences at top of file):\n  \
                  title, subtitle, author, date, theme, aspect, layout, font, logo,\n  \
                  toc, transition, transition_duration, direction, math, math_macros,\n  \
                  math_scale, math_block_align, math_max_height, code_theme, code_columns, doc_style,\n  \
                  break_mode, break_fill, table_fit\n\n\
                  CLI flags override front-matter; front-matter overrides defaults.",
    after_help = "Examples:\n  \
                  md2any talk.md                              → talk.pptx\n  \
                  md2any talk.md -o talk.odp                  → ODP\n  \
                  md2any talk.md -o slides.pdf                → PDF\n  \
                  md2any talk.md -o handout.docx              → Word document\n  \
                  md2any talk.md -o handout.docx --doc-style speaker-notes\n  \
                  md2any talk.md -o notes.odt                 → LibreOffice Writer\n  \
                  md2any talk.md -o review.html               → standalone browser deck\n  \
                  md2any talk.md --format svg -o slides-svg/  → one SVG per slide\n  \
                  md2any talk.md --format png -o slides-png/  → one PNG per slide\n  \
                  md2any talk.md --theme dark --layout studio\n  \
                  md2any talk.md --aspect 9:16                → portrait\n  \
                  md2any talk.md --break-fill 85              → earlier slide breaks\n  \
                  md2any talk.md --table-fit transpose        → force portrait-friendly tables\n  \
                  md2any talk.md --code-theme match           → code follows main theme\n  \
                  md2any talk.md --code-columns two-up        → split eligible code left/right\n  \
                  md2any talk.md --emit-plan plan.json --trace-layout\n  \
                  md2any talk.md --handout 4 -o handout.pdf   → 4-up A4 handout\n  \
                  md2any talk.md --speaker-package out/       → deck + notes + handout\n  \
                  md2any talk.md --font-audit                 → report PDF glyph coverage\n  \
                  md2any talk.md --serve                      → live PDF preview server\n  \
                  md2any talk.md --serve --serve-format html  → live HTML preview\n  \
                  md2any talk.md --serve --serve-format png   → live PNG slide preview\n  \
                  md2any --help-pdf --theme dark              → user manual as dark PDF\n  \
                  md2any --help-md > HELP.md                  → dump the manual source\n  \
                  md2any doctor                               → probe optional CLIs + fonts\n  \
                  md2any licenses                             → print bundled-font licence\n\n\
                  Output formats:\n  \
                  pptx  → PowerPoint OOXML; opens in PowerPoint, Keynote, LibreOffice, Slides\n  \
                  odp   → OpenDocument Impress; opens in LibreOffice, Keynote, Slides\n  \
                  pdf   → PDF 1.7; self-contained — embeds DejaVu Sans, optional --cjk\n  \
                  docx  → Microsoft Word; opens in Word, LibreOffice Writer, Pages, Docs\n  \
                  odt   → OpenDocument Writer; opens in LibreOffice, Word, Docs\n  \
                  html  → Standalone browser deck; keyboard navigation, print-friendly\n\n\
                  svg   → Directory of one SVG file per slide\n  \
                  png   → Directory of one PNG file per slide (requires the svg feature)\n\n\
                  Layouts (each composes with light + dark theme + every aspect):\n  \
                  clean   minimal title underline, full-bleed content (default)\n  \
                  studio  editorial — vertical rail, italic titles, big section numerals\n  \
                  frame   sidebar holds deck title and slide number\n  \
                  bold    magazine — full-width accent block behind every title\n\n\
                  Code blocks support 40+ language tags including: rust, python, js, ts, go,\n  \
                  c, cpp, java, kotlin, scala, csharp, haskell, bcpl, ruby, bash, powershell,\n  \
                  sql, graphql, http, dockerfile, terraform, json, yaml, toml, html, css,\n  \
                  vue, svelte, markdown, diff, bf, cobol, jcl, rexx, pl1/pli/plx, hlasm, db2,\n  \
                  rpg/rpg2/rpg3, rpgle/rpgfree, cl/clle.\n  \
                  Use ` ```rust src/main.rs ` to add a filename caption.\n  \
                  Blocks > 5 lines get line numbers automatically; continuation chunks keep\n  \
                  their original source line numbers."
)]
struct Cli {
    /// Input markdown file(s) — pass `-` to read from stdin.
    /// Multiple files are concatenated in order; front-matter from the first applies.
    /// Omit when using --help-pptx, --help-odp, --help-pdf, --help-docx, --help-odt, --help-html, --help-svg, --help-png, or --help-md.
    #[arg(value_name = "INPUT")]
    inputs: Vec<PathBuf>,

    /// Output file. For svg/png, this is an output directory.
    /// Defaults to input extension or to input-svg/input-png for image sequences.
    #[arg(short, long, value_name = "PATH")]
    output: Option<PathBuf>,

    /// Output format: pptx | odp | pdf | docx | odt | html | svg | png (default: inferred from -o, else pptx)
    #[arg(long, value_name = "NAME")]
    format: Option<String>,

    /// DOCX/ODT document profile: plain | report | handout | speaker-notes.
    /// Defaults to front-matter `doc_style`, then report.
    #[arg(long, value_enum, value_name = "STYLE")]
    doc_style: Option<DocStyleArg>,

    /// Theme: light | dark (applies to all output formats)
    #[arg(long, value_name = "NAME")]
    theme: Option<String>,

    /// Aspect: 16:9, 4:3, 9:16, a4, a4-landscape, a3, a5, letter,
    /// letter-landscape, legal, tabloid, or custom `WxH[unit]`
    /// (e.g. 1920x1080, 300x200mm, 13.3x7.5in).
    #[arg(long, value_name = "RATIO")]
    aspect: Option<String>,

    /// Layout: clean | studio | frame | bold
    #[arg(long, value_name = "NAME")]
    layout: Option<String>,

    /// Continuation splitting: smart | simple | off.
    /// smart splits overlong paragraphs, lists, code, tables, and columns at readable boundaries;
    /// simple keeps paragraphs/lists/columns atomic but still chunks long code/tables;
    /// off disables automatic continuation splitting.
    #[arg(long, value_enum, value_name = "MODE")]
    break_mode: Option<BreakModeArg>,

    /// Percentage of estimated slide content area to fill before breaking (50-120).
    /// Lower values produce more, airier slides; higher values pack more content per slide.
    #[arg(long, value_name = "PCT")]
    break_fill: Option<f32>,

    /// Wide-table handling: auto | split | transpose | off.
    /// auto transposes compact portrait tables and splits very wide tables.
    #[arg(long, value_enum, value_name = "MODE")]
    table_fit: Option<TableFitArg>,

    /// Code block palette: dark | light | match.
    /// Defaults to dark, independent of --theme. match restores the old behavior where code follows --theme.
    #[arg(long, value_enum, value_name = "MODE")]
    code_theme: Option<CodeThemeArg>,

    /// Code block column flow: single | auto | two-up.
    /// two-up applies only to landscape slide outputs; per-fence `columns=...` overrides this.
    #[arg(long, value_enum, value_name = "MODE")]
    code_columns: Option<CodeColumnsArg>,

    /// Math preprocessing: unicode | source | svg.
    /// unicode translates a small LaTeX subset; source leaves math untouched; svg uses built-in display layout.
    #[arg(long, value_enum, value_name = "MODE")]
    math: Option<MathModeArg>,

    /// Scale generated display math in --math svg mode (0.35-3.0, default 1.0).
    #[arg(long, value_name = "N")]
    math_scale: Option<f32>,

    /// Align generated display math blocks: left | center | right.
    #[arg(long, value_enum, value_name = "ALIGN")]
    math_block_align: Option<MathBlockAlignArg>,

    /// Maximum generated display math height in logical SVG pixels.
    #[arg(long, value_name = "PX")]
    math_max_height: Option<f32>,

    /// Override deck title (otherwise from front-matter `title` or input filename)
    #[arg(long, value_name = "STRING")]
    title: Option<String>,

    /// Override deck author (otherwise from front-matter `author`)
    #[arg(long, value_name = "STRING")]
    author: Option<String>,

    /// Body/title font for PPTX/ODP/DOCX/ODT (default Calibri / Calibri Light). PDF always uses the embedded DejaVu Sans.
    #[arg(long, value_name = "NAME")]
    font: Option<String>,

    /// Path to a logo image (PNG or JPEG) rendered in the footer of every content slide.
    /// Overrides `logo:` from front-matter.
    #[arg(long, value_name = "PATH")]
    logo: Option<PathBuf>,

    /// Directory used to cache remote images fetched from http(s) Markdown URLs.
    /// Defaults to the platform cache directory.
    #[arg(
        long,
        value_name = "PATH",
        conflicts_with = "no_remote_image_cache",
        help_heading = "Images"
    )]
    remote_image_cache: Option<PathBuf>,

    /// Disable the remote image cache and fetch http(s) image URLs on every render.
    #[arg(long, help_heading = "Images")]
    no_remote_image_cache: bool,

    /// User-Agent sent when fetching remote images.
    #[arg(long, value_name = "STRING", help_heading = "Images")]
    remote_image_user_agent: Option<String>,

    /// YAML file with theme colour/font/size overrides applied on top of the base theme.
    #[arg(long, value_name = "PATH")]
    theme_file: Option<PathBuf>,

    /// Quiet — suppress the post-render summary line
    #[arg(short, long)]
    quiet: bool,

    /// Lint mode — parse + paginate, then print warnings about likely
    /// visual issues (long titles, narrow tables, walls of text, …) and exit
    /// without writing an output file. Exit code 2 if any warnings.
    #[arg(long)]
    check: bool,

    /// Parse + paginate, then dump a one-line-per-slide outline (page,
    /// kind, block summary, notes/bg flags) and exit. Useful for debugging
    /// pagination, headings, and TOC behaviour without writing output.
    #[arg(long)]
    outline: bool,

    /// Write the post-parse/post-pagination slide IR as JSON.
    #[arg(long, value_name = "PATH")]
    emit_ir: Option<PathBuf>,

    /// Write the estimated render plan as JSON.
    #[arg(long, value_name = "PATH")]
    emit_plan: Option<PathBuf>,

    /// Print a human-readable render-plan summary to stderr.
    #[arg(long)]
    trace_layout: bool,

    /// Watch the markdown input file(s) and rebuild on change. External
    /// dependencies (images, theme-file, logo) are not tracked — restart
    /// the command if those change. Exits cleanly on Ctrl-C.
    #[arg(long)]
    watch: bool,

    /// Produce a handout PDF with N slides per A4 portrait page (2, 4, or 6).
    /// Only affects PDF output; slide layout/theme are unchanged.
    #[arg(long, value_name = "N")]
    handout: Option<u32>,

    /// Produce a presenter-notes PDF: one page per slide with the slide
    /// thumbnail and speaker notes. PDF only.
    #[arg(long, conflicts_with = "handout")]
    with_notes: bool,

    /// Page size for --with-notes PDFs: slide | a4.
    /// slide uses the deck aspect and layout space; a4 keeps the old print-oriented notes pages.
    #[arg(long, value_enum, value_name = "SIZE", default_value = "slide")]
    notes_page_size: NotesPageSizeArg,

    /// Layout for --with-notes PDFs: auto | below | side-by-side.
    /// auto uses side-by-side on wide pages and below on portrait/tall pages.
    #[arg(long, value_enum, value_name = "LAYOUT", default_value = "auto")]
    notes_layout: NotesLayoutArg,

    /// Write a speaker package directory containing the deck, notes PDF,
    /// handout PDF, and a JSON manifest. Deck format defaults to PPTX and
    /// may be overridden with --format pptx|odp|pdf or by -o EXT.
    #[arg(long, value_name = "DIR")]
    speaker_package: Option<PathBuf>,

    /// PDF only: replace bundled DejaVu Sans with a user-supplied TTF/OTF.
    /// The same face is used for regular/bold/italic PDF sans slots.
    #[arg(long, value_name = "PATH")]
    pdf_font: Option<PathBuf>,

    /// PDF only: replace bundled DejaVu Sans Mono for code/monospace text.
    #[arg(long, value_name = "PATH")]
    pdf_mono_font: Option<PathBuf>,

    /// PDF only: comma-separated TTF/OTF fallback font paths tried when the
    /// primary PDF font cannot render a glyph.
    #[arg(long, value_name = "PATH[,PATH...]")]
    font_fallback: Option<String>,

    /// Parse + paginate, then audit PDF font coverage and exit.
    /// Exits with code 2 if any visible glyph has no primary or fallback font.
    #[arg(long)]
    font_audit: bool,

    /// Path to a CJK font (TTF/TTC/OTF) used as a glyph fallback in PDF
    /// output. Activates whenever a text run contains Chinese/Japanese/
    /// Korean characters that the bundled DejaVu font doesn't cover. Try
    /// `--cjk /usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc`.
    #[arg(long, value_name = "PATH")]
    cjk: Option<PathBuf>,

    /// Start a local HTTP preview server with hot reload.
    /// The selected --serve-format is rebuilt whenever any input file changes.
    #[arg(long)]
    serve: bool,

    /// Preview format for --serve: pdf | html | svg | png.
    /// Defaults to pdf and is intentionally independent of --format.
    #[arg(long, value_enum, value_name = "FORMAT", default_value = "pdf")]
    serve_format: ServeFormatArg,

    /// Port for `--serve` (default 8421).
    #[arg(long, value_name = "PORT", default_value_t = 8421)]
    port: u16,

    /// With `--serve`, open the in-browser editor: edit the markdown and see a
    /// live preview side by side. Edits are saved back to the input file.
    #[arg(long)]
    edit: bool,

    /// Draft a deck with an AI model from a prompt, then render it. Reads the
    /// API key from $MD2ANY_API_KEY (or $OPENAI_API_KEY). No input file needed.
    #[arg(long, value_name = "PROMPT", help_heading = "AI")]
    generate: Option<String>,

    /// Chat-completions endpoint for --generate (OpenAI-compatible).
    #[arg(long, value_name = "URL", help_heading = "AI")]
    ai_endpoint: Option<String>,

    /// Model name for --generate.
    #[arg(long, value_name = "MODEL", help_heading = "AI")]
    ai_model: Option<String>,

    /// With --generate, also write the drafted markdown to this file.
    #[arg(long, value_name = "PATH", help_heading = "AI")]
    save_md: Option<PathBuf>,

    /// Build the embedded user manual as PPTX (respects --theme, --layout, --aspect, -o)
    #[arg(long, conflicts_with_all = ["help_odp", "help_pdf", "help_md", "help_docx", "help_odt", "help_html", "help_svg", "help_png"], help_heading = "Manual")]
    help_pptx: bool,

    /// Build the embedded user manual as ODP (respects --theme, --layout, --aspect, -o)
    #[arg(long, conflicts_with_all = ["help_pptx", "help_pdf", "help_md", "help_docx", "help_odt", "help_html", "help_svg", "help_png"], help_heading = "Manual")]
    help_odp: bool,

    /// Build the embedded user manual as PDF (respects --theme, --layout, --aspect, -o)
    #[arg(long, conflicts_with_all = ["help_pptx", "help_odp", "help_md", "help_docx", "help_odt", "help_html", "help_svg", "help_png"], help_heading = "Manual")]
    help_pdf: bool,

    /// Build the embedded user manual as Word DOCX (respects --theme, -o)
    #[arg(long, conflicts_with_all = ["help_pptx", "help_odp", "help_pdf", "help_md", "help_odt", "help_html", "help_svg", "help_png"], help_heading = "Manual")]
    help_docx: bool,

    /// Build the embedded user manual as LibreOffice ODT (respects --theme, -o)
    #[arg(long, conflicts_with_all = ["help_pptx", "help_odp", "help_pdf", "help_md", "help_docx", "help_html", "help_svg", "help_png"], help_heading = "Manual")]
    help_odt: bool,

    /// Build the embedded user manual as standalone HTML (respects --theme, --layout, --aspect, -o)
    #[arg(long, conflicts_with_all = ["help_pptx", "help_odp", "help_pdf", "help_md", "help_docx", "help_odt", "help_svg", "help_png"], help_heading = "Manual")]
    help_html: bool,

    /// Build the embedded user manual as one SVG per slide (respects --theme, --layout, --aspect, -o)
    #[arg(long, conflicts_with_all = ["help_pptx", "help_odp", "help_pdf", "help_md", "help_docx", "help_odt", "help_html", "help_png"], help_heading = "Manual")]
    help_svg: bool,

    /// Build the embedded user manual as one PNG per slide (respects --theme, --layout, --aspect, -o)
    #[arg(long, conflicts_with_all = ["help_pptx", "help_odp", "help_pdf", "help_md", "help_docx", "help_odt", "help_html", "help_svg"], help_heading = "Manual")]
    help_png: bool,

    /// Print the embedded user-manual Markdown source to stdout
    #[arg(long, conflicts_with_all = ["help_pptx", "help_odp", "help_pdf", "help_docx", "help_odt", "help_html", "help_svg", "help_png"], help_heading = "Manual")]
    help_md: bool,

    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Write a starter markdown file with front-matter and an example structure
    New {
        /// Path to create (e.g. talk.md). Refuses to overwrite an existing file.
        path: PathBuf,
        /// Skip the existence check and overwrite
        #[arg(long)]
        force: bool,
    },
    /// Probe the environment: optional CLIs, bundled fonts, build features.
    /// Use this when diagrams or CJK glyphs aren't rendering as expected.
    Doctor,
    /// Print the licence notice for the fonts embedded in the binary.
    /// The DejaVu / Bitstream Vera / Arev font licences require the
    /// notice to travel with the font programs — since the fonts are
    /// embedded in the executable, this command is the canonical way to
    /// surface that notice for binary-only distributions.
    Licenses,
}

const HELP_MD: &str = include_str!("../HELP.md");

const STARTER_TEMPLATE: &str = include_str!("../templates/starter.md");

fn main() -> Result<()> {
    let cli = Cli::parse();
    let start = Instant::now();

    if let Some(Cmd::New { path, force }) = &cli.cmd {
        if path.exists() && !*force {
            anyhow::bail!(
                "{} already exists (pass --force to overwrite)",
                path.display()
            );
        }
        std::fs::write(path, STARTER_TEMPLATE)
            .with_context(|| format!("write {}", path.display()))?;
        eprintln!("md2any: wrote starter deck → {}", path.display());
        return Ok(());
    }

    if matches!(cli.cmd, Some(Cmd::Doctor)) {
        run_doctor();
        return Ok(());
    }

    if matches!(cli.cmd, Some(Cmd::Licenses)) {
        use std::io::Write;
        // Embedded at compile time so the notice always ships with the
        // executable, satisfying the DejaVu / Bitstream Vera / Arev
        // requirement that the notice travel with the font programs.
        const FONT_LICENSE: &str = include_str!("../assets/fonts/LICENSE.md");
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(FONT_LICENSE.as_bytes());
        return Ok(());
    }

    if cli.help_md {
        use std::io::Write;
        std::io::stdout().write_all(HELP_MD.as_bytes())?;
        return Ok(());
    }

    image::configure_remote_images(image::RemoteImageOptions {
        cache_enabled: !cli.no_remote_image_cache,
        cache_dir: cli.remote_image_cache.clone(),
        user_agent: cli.remote_image_user_agent.clone(),
    });

    let help_format = if cli.help_pptx {
        Some(Format::Pptx)
    } else if cli.help_odp {
        Some(Format::Odp)
    } else if cli.help_pdf {
        Some(Format::Pdf)
    } else if cli.help_docx {
        Some(Format::Docx)
    } else if cli.help_odt {
        Some(Format::Odt)
    } else if cli.help_html {
        Some(Format::Html)
    } else if cli.help_svg {
        Some(Format::Svg)
    } else if cli.help_png {
        Some(Format::Png)
    } else {
        None
    };

    if cli.generate.is_some() && (cli.serve || cli.watch) {
        anyhow::bail!(
            "--generate is a one-shot render; to edit the result, use --generate … --save-md deck.md, then --serve deck.md"
        );
    }

    let (input_source, input_path): (String, PathBuf) = if let Some(prompt) = &cli.generate {
        let opts = md2any::ai::AiOptions::from_env(cli.ai_endpoint.clone(), cli.ai_model.clone())
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        eprintln!(
            "md2any: drafting deck via {} ({})…",
            opts.endpoint, opts.model
        );
        let md =
            md2any::ai::generate(&opts, prompt).map_err(|e| anyhow::anyhow!("--generate: {e}"))?;
        if let Some(path) = &cli.save_md {
            std::fs::write(path, &md).with_context(|| format!("write {}", path.display()))?;
            eprintln!("md2any: drafted markdown saved to {}", path.display());
        }
        let stem = cli
            .save_md
            .clone()
            .unwrap_or_else(|| PathBuf::from("generated.md"));
        (md, stem)
    } else if let Some(fmt) = help_format {
        let _ = fmt;
        (HELP_MD.to_string(), PathBuf::from("md2any-help.md"))
    } else {
        if cli.inputs.is_empty() {
            anyhow::bail!(
                "no input file (or use --help-pptx, --help-odp, --help-pdf, --help-docx, --help-odt, --help-html, --help-svg, --help-png, --help-md, or pipe with '-')"
            );
        }
        let stdin_count = cli.inputs.iter().filter(|p| p.as_os_str() == "-").count();
        if stdin_count > 0 && cli.inputs.len() > 1 {
            anyhow::bail!("stdin input ('-') cannot be combined with other input files");
        }
        if stdin_count == 1 {
            use std::io::Read;
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .with_context(|| "read stdin")?;
            (buf, PathBuf::from("deck.md"))
        } else if cli.inputs.len() == 1 {
            let path = cli.inputs[0].clone();
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("read {}", path.display()))?;
            (text, path)
        } else {
            // Multi-file: front-matter on the first file applies to the deck;
            // subsequent files contribute body content only (their front-matter,
            // if any, is stripped to avoid leaking YAML into the slide stream).
            let first = &cli.inputs[0];
            let first_text = std::fs::read_to_string(first)
                .with_context(|| format!("read {}", first.display()))?;
            let mut combined = first_text;
            for path in &cli.inputs[1..] {
                let text = std::fs::read_to_string(path)
                    .with_context(|| format!("read {}", path.display()))?;
                let (_, body) = front_matter::extract(&text);
                if !combined.ends_with('\n') {
                    combined.push('\n');
                }
                combined.push('\n');
                combined.push_str(&body);
            }
            (combined, first.clone())
        }
    };
    let input = input_source;

    if cli.watch {
        if help_format.is_some() || cli.check || cli.font_audit {
            anyhow::bail!("--watch is incompatible with --check / --font-audit / --help-* modes");
        }
        if cli.speaker_package.is_some() {
            anyhow::bail!("--watch is incompatible with --speaker-package");
        }
        if cli.inputs.is_empty() {
            anyhow::bail!("--watch requires an input file");
        }
        if cli.inputs.iter().any(|p| p.as_os_str() == "-") {
            anyhow::bail!("--watch cannot be used with stdin input");
        }
        return run_watch(&cli, &input_path);
    }

    if cli.serve {
        if help_format.is_some() || cli.check || cli.font_audit {
            anyhow::bail!("--serve is incompatible with --check / --font-audit / --help-* modes");
        }
        if cli.emit_ir.is_some() || cli.emit_plan.is_some() || cli.trace_layout {
            anyhow::bail!(
                "--serve is incompatible with --emit-ir, --emit-plan, and --trace-layout"
            );
        }
        if cli.speaker_package.is_some() {
            anyhow::bail!("--serve is incompatible with --speaker-package");
        }
        if cli.inputs.is_empty() {
            anyhow::bail!("--serve requires an input file");
        }
        if cli.inputs.iter().any(|p| p.as_os_str() == "-") {
            anyhow::bail!("--serve cannot be used with stdin input");
        }
        if cli.edit && cli.inputs.len() != 1 {
            anyhow::bail!("--edit needs exactly one input file to edit");
        }
        return run_serve(&cli);
    }
    if cli.edit {
        anyhow::bail!("--edit requires --serve");
    }

    build_once(&cli, &input_path, &input, help_format, start)
}

fn build_once(
    cli: &Cli,
    input_path: &Path,
    input: &str,
    help_format: Option<Format>,
    start: Instant,
) -> Result<()> {
    let (front, body) = front_matter::extract(input);

    let theme_name = cli
        .theme
        .clone()
        .or_else(|| front.theme.clone())
        .unwrap_or_else(|| "light".into());
    let aspect = cli
        .aspect
        .clone()
        .or_else(|| front.aspect.clone())
        .unwrap_or_else(|| "16:9".into());
    let font_pref = cli.font.clone().or_else(|| front.font.clone());
    let mut theme = theme::Theme::resolve(&theme_name, &aspect, font_pref.as_deref())
        .with_context(|| "resolve theme")?;
    theme.apply_code_theme(resolve_code_theme(cli.code_theme, &front)?);
    let theme_file_dir = cli
        .theme_file
        .as_ref()
        .and_then(|p| p.parent())
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_path_buf());
    if let Some(path) = &cli.theme_file {
        let ov = theme::load_override(path)?;
        theme
            .apply_override(&ov)
            .with_context(|| format!("apply theme overlay {}", path.display()))?;
    }

    let layout_name = cli
        .layout
        .clone()
        .or_else(|| front.layout.clone())
        .unwrap_or_else(|| "clean".into());
    let layout = layout::Layout::resolve(&layout_name).with_context(|| "resolve layout")?;
    let pagination = resolve_pagination_options(
        cli.break_mode,
        cli.break_fill,
        cli.table_fit,
        cli.code_columns,
        &front,
    )?;
    let math_mode = resolve_math_mode(cli.math, &front)?;
    let math_svg = resolve_math_svg_options(
        cli.math_scale,
        cli.math_block_align,
        cli.math_max_height,
        &front,
    )?;
    let doc_options = resolve_document_options(cli.doc_style, &front)?;

    let stem_title = input_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Presentation".into());
    let deck_title = cli
        .title
        .clone()
        .or_else(|| front.title.clone())
        .unwrap_or(stem_title);
    let author = cli
        .author
        .clone()
        .or_else(|| front.author.clone())
        .unwrap_or_else(|| "md2any".into());
    let base_dir = input_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    let mut parsed = parser::parse_with_options(
        &body,
        &front,
        &deck_title,
        parser::ParseOptions {
            math_mode,
            math_svg,
            include_base_dir: Some(base_dir.clone()),
            ..Default::default()
        },
    );
    // Render diagram code blocks (dot/mermaid/plantuml) to PNGs if the
    // corresponding CLI is available; otherwise leave them as code.
    let deck_stem = input_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "deck".into());
    let cache_dir = diagram::cache_dir_for(&deck_stem);
    let _ = diagram::pre_render(&mut parsed, &cache_dir);
    let slides = paginate::paginate_for_layout_with_options(parsed, &theme, &layout, pagination);
    let slides = if front.toc {
        toc::inject(slides)
    } else {
        slides
    };

    if let Some(path) = &cli.emit_ir {
        write_json(path, &render_plan::ir_dump(&slides))
            .with_context(|| format!("write {}", path.display()))?;
        if !cli.quiet {
            eprintln!("md2any: wrote IR JSON → {}", path.display());
        }
    }

    if cli.emit_plan.is_some() || cli.trace_layout {
        let plan = render_plan::build(
            input_path.display().to_string(),
            deck_title.clone(),
            &slides,
            &theme,
            &layout,
            pagination,
        );
        if cli.trace_layout {
            eprint!("{}", render_plan::trace_text(&plan));
        }
        if let Some(path) = &cli.emit_plan {
            write_json(path, &plan).with_context(|| format!("write {}", path.display()))?;
            if !cli.quiet {
                eprintln!("md2any: wrote render-plan JSON → {}", path.display());
            }
        }
    }

    let pdf_font_paths = resolve_pdf_font_paths(cli, &theme, theme_file_dir.as_deref())?;

    if cli.font_audit {
        let audit = build_font_audit(&slides, &pdf_font_paths)?;
        report_font_audit(&audit, true);
        std::process::exit(if audit.missing.is_empty() { 0 } else { 2 });
    }

    if cli.check {
        let warnings = lint::check(&slides, &theme);
        lint::report(&warnings);
        let math_diagnostics = if matches!(math_mode, md2any::math::MathMode::Unicode) {
            md2any::math::diagnose(&body)
        } else {
            Vec::new()
        };
        report_math_diagnostics(&math_diagnostics);
        let font_missing = if pdf_font_paths.is_configured() {
            let audit = build_font_audit(&slides, &pdf_font_paths)?;
            report_font_audit(&audit, false);
            audit.missing.len()
        } else {
            0
        };
        std::process::exit(
            if warnings.is_empty() && math_diagnostics.is_empty() && font_missing == 0 {
                0
            } else {
                2
            },
        );
    }

    if cli.outline {
        print_outline(&slides);
        return Ok(());
    }

    let format = if let Some(fmt) = help_format {
        fmt
    } else if let Some(name) = &cli.format {
        Format::from_name(name)?
    } else if let Some(out) = &cli.output {
        out.extension()
            .and_then(|e| Format::from_extension(&e.to_string_lossy()))
            .unwrap_or(Format::Pptx)
    } else {
        Format::Pptx
    };

    let output = cli
        .output
        .clone()
        .unwrap_or_else(|| default_output_path(format, input_path, help_format.is_some()));

    let n_slides = slides.len();

    let logo_path: Option<PathBuf> = if let Some(p) = &cli.logo {
        // CLI path is relative to the user's CWD; use as-is unless it doesn't
        // exist there, in which case try base_dir.
        if p.exists() || p.is_absolute() {
            Some(p.clone())
        } else {
            Some(base_dir.join(p))
        }
    } else if let Some(p) = &front.logo {
        let p = PathBuf::from(p);
        if p.is_absolute() {
            Some(p)
        } else {
            Some(base_dir.join(p))
        }
    } else {
        None
    };

    let transition = front.transition.as_deref();
    let trans_dur = front.transition_duration.unwrap_or(0.4);
    let direction = front.direction.as_deref();
    let pdf_options = PdfRenderOptions {
        handout: cli.handout,
        with_notes: cli.with_notes,
        notes_page_size: cli.notes_page_size.into(),
        notes_layout: cli.notes_layout.into(),
        font_options: pdf_font_paths.as_options(),
    };

    if let Some(dir) = &cli.speaker_package {
        write_speaker_package(
            dir,
            format,
            &output,
            input_path,
            &slides,
            &theme,
            &layout,
            &deck_title,
            &author,
            &base_dir,
            logo_path.as_deref(),
            transition,
            trans_dur,
            direction,
            &aspect,
            &pdf_options,
            start,
            cli.quiet,
        )?;
        return Ok(());
    }

    if format.is_slide_image_sequence() {
        let files = render_slide_image_files(
            format,
            &slides,
            &theme,
            &layout,
            &deck_title,
            &author,
            &base_dir,
            logo_path.as_deref(),
            direction,
        )?;
        write_slide_image_directory(&output, &files)?;
        if !cli.quiet {
            let elapsed = start.elapsed();
            let bytes: usize = files.iter().map(|f| f.bytes.len()).sum();
            eprintln!(
                "md2any: {n_slides} slides → {out} ({count} {format} files, {size} KB, {ms} ms, {theme}, {aspect}, {layout})",
                out = output.display(),
                count = files.len(),
                format = format.name(),
                size = bytes / 1024,
                ms = elapsed.as_millis(),
                theme = theme.name,
                aspect = aspect,
                layout = layout.name(),
            );
        }
        return Ok(());
    }

    let bytes = render_format_bytes(
        format,
        &slides,
        &theme,
        &layout,
        &deck_title,
        &author,
        &base_dir,
        logo_path.as_deref(),
        transition,
        trans_dur,
        direction,
        &pdf_options,
        &doc_options,
    )?;

    std::fs::write(&output, &bytes).with_context(|| format!("write {}", output.display()))?;

    if !cli.quiet {
        let elapsed = start.elapsed();
        eprintln!(
            "md2any: {n_slides} slides → {out} ({size} KB, {ms} ms, {format}, {theme}, {aspect}, {layout})",
            out = output.display(),
            size = bytes.len() / 1024,
            ms = elapsed.as_millis(),
            format = format.name(),
            theme = theme.name,
            aspect = aspect,
            layout = layout.name(),
        );
    }

    Ok(())
}

fn default_output_path(format: Format, input_path: &Path, help: bool) -> PathBuf {
    if help {
        if format.is_slide_image_sequence() {
            PathBuf::from(format!("md2any-help-{}", format.ext()))
        } else {
            PathBuf::from(format!("md2any-help.{}", format.ext()))
        }
    } else {
        let stem = input_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "deck".to_string());
        if format.is_slide_image_sequence() {
            input_path.with_file_name(format!("{stem}-{}", format.ext()))
        } else {
            let mut p = input_path.to_path_buf();
            p.set_extension(format.ext());
            p
        }
    }
}

fn render_slide_image_files(
    format: Format,
    slides: &[md2any::ir::Slide],
    theme: &theme::Theme,
    layout: &layout::Layout,
    deck_title: &str,
    author: &str,
    base_dir: &Path,
    logo: Option<&Path>,
    direction: Option<&str>,
) -> Result<Vec<svg::SlideFile>> {
    let image_format = match format {
        Format::Svg => svg::ImageFormat::Svg,
        Format::Png => svg::ImageFormat::Png,
        _ => anyhow::bail!("{} is not a slide-image sequence format", format.name()),
    };
    svg::write_files(
        slides,
        theme,
        layout,
        deck_title,
        author,
        base_dir,
        logo,
        direction,
        image_format,
    )
}

fn write_slide_image_directory(dir: &Path, files: &[svg::SlideFile]) -> Result<()> {
    if dir.exists() && !dir.is_dir() {
        anyhow::bail!(
            "slide-image output path must be a directory for svg/png formats: {}",
            dir.display()
        );
    }
    std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    for file in files {
        let path = dir.join(&file.name);
        std::fs::write(&path, &file.bytes).with_context(|| format!("write {}", path.display()))?;
    }
    Ok(())
}

#[derive(Clone)]
struct PdfRenderOptions<'a> {
    handout: Option<u32>,
    with_notes: bool,
    notes_page_size: pdf::NotesPageSize,
    notes_layout: pdf::NotesLayout,
    font_options: md2any::font::PdfFontOptions<'a>,
}

#[derive(Debug, Default, Clone)]
struct PdfFontPaths {
    pdf_font: Option<PathBuf>,
    pdf_mono_font: Option<PathBuf>,
    fallback_fonts: Vec<PathBuf>,
}

impl PdfFontPaths {
    fn as_options(&self) -> md2any::font::PdfFontOptions<'_> {
        md2any::font::PdfFontOptions {
            pdf_font: self.pdf_font.as_deref(),
            pdf_mono_font: self.pdf_mono_font.as_deref(),
            fallback_fonts: self.fallback_fonts.iter().map(|p| p.as_path()).collect(),
        }
    }

    fn is_configured(&self) -> bool {
        self.pdf_font.is_some() || self.pdf_mono_font.is_some() || !self.fallback_fonts.is_empty()
    }
}

fn resolve_pdf_font_paths(
    cli: &Cli,
    theme: &theme::Theme,
    theme_file_dir: Option<&Path>,
) -> Result<PdfFontPaths> {
    resolve_pdf_font_paths_from_parts(
        cli.pdf_font.as_ref(),
        cli.pdf_mono_font.as_ref(),
        cli.font_fallback.as_deref(),
        cli.cjk.as_ref(),
        theme,
        theme_file_dir,
    )
}

fn resolve_pdf_font_paths_from_parts(
    cli_pdf_font: Option<&PathBuf>,
    cli_pdf_mono_font: Option<&PathBuf>,
    cli_font_fallback: Option<&str>,
    cli_cjk: Option<&PathBuf>,
    theme: &theme::Theme,
    theme_file_dir: Option<&Path>,
) -> Result<PdfFontPaths> {
    let pdf_font = cli_pdf_font.cloned().or_else(|| {
        theme
            .pdf_font
            .as_deref()
            .map(|p| resolve_theme_asset_path(p, theme_file_dir))
    });
    let pdf_mono_font = cli_pdf_mono_font.cloned().or_else(|| {
        theme
            .pdf_mono_font
            .as_deref()
            .map(|p| resolve_theme_asset_path(p, theme_file_dir))
    });
    let mut fallback_fonts: Vec<PathBuf> = if let Some(value) = cli_font_fallback {
        theme::split_font_fallbacks(value)
            .into_iter()
            .map(PathBuf::from)
            .collect()
    } else {
        theme
            .font_fallback
            .iter()
            .map(|p| resolve_theme_asset_path(p, theme_file_dir))
            .collect()
    };
    if let Some(path) = cli_cjk {
        fallback_fonts.push(path.clone());
    }
    Ok(PdfFontPaths {
        pdf_font,
        pdf_mono_font,
        fallback_fonts,
    })
}

fn resolve_theme_asset_path(value: &str, theme_file_dir: Option<&Path>) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() || path.exists() {
        return path;
    }
    if let Some(dir) = theme_file_dir {
        return dir.join(path);
    }
    path
}

fn build_font_audit(
    slides: &[md2any::ir::Slide],
    paths: &PdfFontPaths,
) -> Result<md2any::font::FontAudit> {
    let fonts = md2any::font::PdfFonts::load_with_options(paths.as_options())?;
    Ok(md2any::font::audit_pdf_fonts(slides, &fonts))
}

fn report_font_audit(audit: &md2any::font::FontAudit, verbose: bool) {
    if verbose {
        eprintln!(
            "md2any font audit: {} slide(s), {} PDF face(s)",
            audit.slide_count,
            audit.face_names.len()
        );
        for (idx, name) in audit.face_names.iter().enumerate() {
            eprintln!("  F{} {}", idx + 1, name);
        }
        if audit.fallback_hits.is_empty() {
            eprintln!("  fallback: no glyphs needed fallback fonts");
        } else {
            eprintln!("  fallback: {} unique glyph(s)", audit.fallback_hits.len());
            for hit in audit.fallback_hits.iter().take(20) {
                eprintln!(
                    "    {} {} via {} on slide {} ({}, {} use{})",
                    display_char(hit.ch),
                    hit.codepoint,
                    hit.face.as_deref().unwrap_or("fallback"),
                    hit.first_slide,
                    hit.context,
                    hit.count,
                    if hit.count == 1 { "" } else { "s" }
                );
            }
            if audit.fallback_hits.len() > 20 {
                eprintln!("    ... {} more", audit.fallback_hits.len() - 20);
            }
        }
    }

    if audit.missing.is_empty() {
        if verbose {
            eprintln!("  missing: none");
        }
        return;
    }

    eprintln!(
        "md2any font audit: {} missing glyph(s)",
        audit.missing.len()
    );
    for hit in audit.missing.iter().take(30) {
        eprintln!(
            "  slide {}: [{}-font-missing] {} {} in {} ({} use{})",
            hit.first_slide,
            hit.primary,
            display_char(hit.ch),
            hit.codepoint,
            hit.context,
            hit.count,
            if hit.count == 1 { "" } else { "s" }
        );
    }
    if audit.missing.len() > 30 {
        eprintln!("  ... {} more", audit.missing.len() - 30);
    }
}

fn display_char(ch: char) -> String {
    match ch {
        ' ' => "' '".to_string(),
        _ => ch.escape_default().to_string(),
    }
}

fn render_format_bytes(
    format: Format,
    slides: &[md2any::ir::Slide],
    theme: &theme::Theme,
    layout: &layout::Layout,
    deck_title: &str,
    author: &str,
    base_dir: &Path,
    logo: Option<&Path>,
    transition: Option<&str>,
    trans_dur: f32,
    direction: Option<&str>,
    pdf_options: &PdfRenderOptions<'_>,
    doc_options: &document::DocumentOptions,
) -> Result<Vec<u8>> {
    match format {
        Format::Pptx => pptx::write(
            slides, theme, layout, deck_title, author, base_dir, logo, transition, trans_dur,
            direction,
        ),
        Format::Odp => odp::write(
            slides, theme, layout, deck_title, author, base_dir, logo, transition, trans_dur,
            direction,
        ),
        Format::Pdf => pdf::write_with_font_options(
            slides,
            theme,
            layout,
            deck_title,
            author,
            base_dir,
            logo,
            pdf_options.handout,
            transition,
            trans_dur,
            direction,
            pdf_options.with_notes,
            pdf_options.notes_page_size,
            pdf_options.notes_layout,
            pdf_options.font_options.clone(),
        ),
        Format::Odt => odt::write_with_options(
            slides,
            theme,
            deck_title,
            author,
            base_dir,
            logo,
            direction,
            doc_options,
        ),
        Format::Docx => docx::write_with_options(
            slides,
            theme,
            deck_title,
            author,
            base_dir,
            logo,
            direction,
            doc_options,
        ),
        Format::Html => html::write(
            slides, theme, layout, deck_title, author, base_dir, logo, direction,
        ),
        Format::Svg | Format::Png => {
            anyhow::bail!(
                "{} writes a slide-image directory, not a single file",
                format.name()
            )
        }
    }
}

#[derive(serde::Serialize)]
struct SpeakerPackageManifest {
    schema: &'static str,
    source: String,
    title: String,
    author: String,
    slide_count: usize,
    notes_count: usize,
    theme: String,
    aspect: String,
    layout: String,
    deck_format: String,
    notes_page_size: String,
    notes_layout: String,
    handout_slides_per_page: u32,
    assets: Vec<String>,
    artifacts: Vec<SpeakerPackageArtifact>,
}

#[derive(serde::Serialize)]
struct SpeakerPackageArtifact {
    kind: &'static str,
    path: String,
    format: String,
    bytes: usize,
}

fn write_speaker_package(
    dir: &Path,
    deck_format: Format,
    output: &Path,
    input_path: &Path,
    slides: &[md2any::ir::Slide],
    theme: &theme::Theme,
    layout: &layout::Layout,
    deck_title: &str,
    author: &str,
    base_dir: &Path,
    logo: Option<&Path>,
    transition: Option<&str>,
    trans_dur: f32,
    direction: Option<&str>,
    aspect: &str,
    pdf_options: &PdfRenderOptions<'_>,
    start: Instant,
    quiet: bool,
) -> Result<()> {
    if !matches!(deck_format, Format::Pptx | Format::Odp | Format::Pdf) {
        anyhow::bail!(
            "--speaker-package deck format must be pptx, odp, or pdf (got {})",
            deck_format.name()
        );
    }
    let handout_n = pdf_options.handout.unwrap_or(4);
    if !matches!(handout_n, 2 | 4 | 6) {
        anyhow::bail!(
            "--speaker-package handout supports 2, 4, or 6 slides per page (got {})",
            handout_n
        );
    }

    std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;

    let stem = package_stem(output, input_path);
    let deck_path = dir.join(format!("{}.{}", stem, deck_format.ext()));
    let notes_path = dir.join(format!("{stem}-notes.pdf"));
    let handout_path = dir.join(format!("{stem}-handout.pdf"));
    let manifest_path = dir.join(format!("{stem}-manifest.json"));

    let deck_bytes = render_format_bytes(
        deck_format,
        slides,
        theme,
        layout,
        deck_title,
        author,
        base_dir,
        logo,
        transition,
        trans_dur,
        direction,
        &PdfRenderOptions {
            handout: None,
            with_notes: false,
            notes_page_size: pdf_options.notes_page_size,
            notes_layout: pdf_options.notes_layout,
            font_options: pdf_options.font_options.clone(),
        },
        &document::DocumentOptions::default(),
    )?;
    let notes_bytes = pdf::write_with_font_options(
        slides,
        theme,
        layout,
        deck_title,
        author,
        base_dir,
        logo,
        None,
        transition,
        trans_dur,
        direction,
        true,
        pdf_options.notes_page_size,
        pdf_options.notes_layout,
        pdf_options.font_options.clone(),
    )?;
    let handout_bytes = pdf::write_with_font_options(
        slides,
        theme,
        layout,
        deck_title,
        author,
        base_dir,
        logo,
        Some(handout_n),
        transition,
        trans_dur,
        direction,
        false,
        pdf_options.notes_page_size,
        pdf_options.notes_layout,
        pdf_options.font_options.clone(),
    )?;

    std::fs::write(&deck_path, &deck_bytes)
        .with_context(|| format!("write {}", deck_path.display()))?;
    std::fs::write(&notes_path, &notes_bytes)
        .with_context(|| format!("write {}", notes_path.display()))?;
    std::fs::write(&handout_path, &handout_bytes)
        .with_context(|| format!("write {}", handout_path.display()))?;

    let artifacts = vec![
        SpeakerPackageArtifact {
            kind: "deck",
            path: package_file_name(&deck_path),
            format: deck_format.name().to_string(),
            bytes: deck_bytes.len(),
        },
        SpeakerPackageArtifact {
            kind: "notes",
            path: package_file_name(&notes_path),
            format: "pdf".to_string(),
            bytes: notes_bytes.len(),
        },
        SpeakerPackageArtifact {
            kind: "handout",
            path: package_file_name(&handout_path),
            format: "pdf".to_string(),
            bytes: handout_bytes.len(),
        },
    ];
    let manifest = SpeakerPackageManifest {
        schema: "md2any-speaker-package-v1",
        source: input_path.display().to_string(),
        title: deck_title.to_string(),
        author: author.to_string(),
        slide_count: slides.len(),
        notes_count: slides.iter().filter(|s| s.notes.is_some()).count(),
        theme: theme.name.to_string(),
        aspect: aspect.to_string(),
        layout: layout.name().to_string(),
        deck_format: deck_format.name().to_string(),
        notes_page_size: notes_page_size_name(pdf_options.notes_page_size).to_string(),
        notes_layout: notes_layout_name(pdf_options.notes_layout).to_string(),
        handout_slides_per_page: handout_n,
        assets: manifest_assets(slides, logo),
        artifacts,
    };
    write_json(&manifest_path, &manifest)
        .with_context(|| format!("write {}", manifest_path.display()))?;

    if !quiet {
        let elapsed = start.elapsed();
        eprintln!(
            "md2any: speaker package → {dir} (deck, notes, handout, manifest; {slides} slides, {ms} ms, {format}, {theme}, {aspect}, {layout})",
            dir = dir.display(),
            slides = slides.len(),
            ms = elapsed.as_millis(),
            format = deck_format.name(),
            theme = theme.name,
            aspect = aspect,
            layout = layout.name(),
        );
    }

    Ok(())
}

fn package_stem(output: &Path, input_path: &Path) -> String {
    let raw = output
        .file_stem()
        .or_else(|| input_path.file_stem())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "deck".to_string());
    sanitize_package_stem(&raw)
}

fn sanitize_package_stem(raw: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in raw.chars() {
        let keep = ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.');
        let mapped = if keep { ch } else { '-' };
        if mapped == '-' {
            if !last_dash {
                out.push(mapped);
            }
            last_dash = true;
        } else {
            out.push(mapped);
            last_dash = false;
        }
    }
    let trimmed = out.trim_matches(|c| matches!(c, '-' | '_' | '.'));
    if trimmed.is_empty() {
        "deck".to_string()
    } else {
        trimmed.to_string()
    }
}

fn package_file_name(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn notes_page_size_name(value: pdf::NotesPageSize) -> &'static str {
    match value {
        pdf::NotesPageSize::Slide => "slide",
        pdf::NotesPageSize::A4 => "a4",
    }
}

fn notes_layout_name(value: pdf::NotesLayout) -> &'static str {
    match value {
        pdf::NotesLayout::Auto => "auto",
        pdf::NotesLayout::Below => "below",
        pdf::NotesLayout::SideBySide => "side-by-side",
    }
}

fn manifest_assets(slides: &[md2any::ir::Slide], logo: Option<&Path>) -> Vec<String> {
    let mut assets = std::collections::BTreeSet::new();
    if let Some(path) = logo {
        assets.insert(path.display().to_string());
    }
    for slide in slides {
        if let Some(bg) = &slide.bg_image {
            assets.insert(bg.clone());
        }
        collect_block_assets(&slide.blocks, &mut assets);
    }
    assets.into_iter().collect()
}

fn collect_block_assets(
    blocks: &[md2any::ir::Block],
    assets: &mut std::collections::BTreeSet<String>,
) {
    for block in blocks {
        match block {
            md2any::ir::Block::Image { src, .. } => {
                assets.insert(src.clone());
            }
            md2any::ir::Block::Columns { left, right } => {
                collect_block_assets(left, assets);
                collect_block_assets(right, assets);
            }
            _ => {}
        }
    }
}

fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    std::fs::write(path, bytes)?;
    Ok(())
}

fn resolve_pagination_options(
    cli_mode: Option<BreakModeArg>,
    cli_fill: Option<f32>,
    cli_table_fit: Option<TableFitArg>,
    cli_code_columns: Option<CodeColumnsArg>,
    front: &md2any::ir::FrontMatter,
) -> Result<paginate::PaginationOptions> {
    let break_mode = if let Some(mode) = cli_mode {
        mode.into()
    } else if let Some(mode) = front.break_mode.as_deref() {
        parse_break_mode(mode)?
    } else {
        paginate::BreakMode::Smart
    };

    let fill_pct = cli_fill.or(front.break_fill).unwrap_or(100.0);
    if !(50.0..=120.0).contains(&fill_pct) {
        anyhow::bail!("break_fill/--break-fill must be between 50 and 120 (got {fill_pct})");
    }
    let table_fit = if let Some(mode) = cli_table_fit {
        mode.into()
    } else if let Some(mode) = front.table_fit.as_deref() {
        parse_table_fit(mode)?
    } else {
        paginate::TableFit::Auto
    };
    let code_columns = if let Some(mode) = cli_code_columns {
        mode.into()
    } else if let Some(mode) = front.code_columns.as_deref() {
        parse_code_columns(mode)?
    } else {
        md2any::ir::CodeColumns::Single
    };

    Ok(paginate::PaginationOptions {
        break_mode,
        fill: fill_pct / 100.0,
        table_fit,
        code_columns,
    })
}

fn parse_break_mode(value: &str) -> Result<paginate::BreakMode> {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "smart" => Ok(paginate::BreakMode::Smart),
        "simple" => Ok(paginate::BreakMode::Simple),
        "off" | "none" | "disabled" => Ok(paginate::BreakMode::Off),
        other => anyhow::bail!("unknown break mode: {other} (try smart, simple, or off)"),
    }
}

fn parse_table_fit(value: &str) -> Result<paginate::TableFit> {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "auto" | "smart" => Ok(paginate::TableFit::Auto),
        "split" | "columns" => Ok(paginate::TableFit::Split),
        "transpose" | "portrait" => Ok(paginate::TableFit::Transpose),
        "off" | "none" | "disabled" => Ok(paginate::TableFit::Off),
        other => anyhow::bail!("unknown table_fit: {other} (try auto, split, transpose, or off)"),
    }
}

fn parse_code_columns(value: &str) -> Result<md2any::ir::CodeColumns> {
    let normalized = value.trim().to_ascii_lowercase().replace('_', "-");
    match normalized.as_str() {
        "1" | "one" | "single" | "off" | "none" => Ok(md2any::ir::CodeColumns::Single),
        "auto" | "smart" => Ok(md2any::ir::CodeColumns::Auto),
        "2" | "two" | "two-up" | "twoup" | "columns" => Ok(md2any::ir::CodeColumns::TwoUp),
        other => anyhow::bail!("unknown code_columns: {other} (try single, auto, or two-up)"),
    }
}

fn resolve_code_theme(
    cli_theme: Option<CodeThemeArg>,
    front: &md2any::ir::FrontMatter,
) -> Result<theme::CodeTheme> {
    if let Some(theme) = cli_theme {
        Ok(theme.into())
    } else if let Some(theme) = front.code_theme.as_deref() {
        parse_code_theme(theme)
    } else {
        Ok(theme::CodeTheme::Dark)
    }
}

fn parse_code_theme(value: &str) -> Result<theme::CodeTheme> {
    let normalized = value.trim().to_ascii_lowercase().replace('_', "-");
    match normalized.as_str() {
        "dark" | "night" => Ok(theme::CodeTheme::Dark),
        "light" => Ok(theme::CodeTheme::Light),
        "match" | "theme" | "main" | "inherit" => Ok(theme::CodeTheme::Match),
        other => anyhow::bail!("unknown code_theme: {other} (try dark, light, or match)"),
    }
}

fn resolve_math_mode(
    cli_mode: Option<MathModeArg>,
    front: &md2any::ir::FrontMatter,
) -> Result<md2any::math::MathMode> {
    if let Some(mode) = cli_mode {
        Ok(mode.into())
    } else if let Some(mode) = front.math.as_deref() {
        parse_math_mode(mode)
    } else {
        Ok(md2any::math::MathMode::Unicode)
    }
}

fn resolve_math_svg_options(
    cli_scale: Option<f32>,
    cli_align: Option<MathBlockAlignArg>,
    cli_max_height: Option<f32>,
    front: &md2any::ir::FrontMatter,
) -> Result<md2any::math::MathSvgOptions> {
    let scale = cli_scale.or(front.math_scale).unwrap_or(1.0);
    if !(0.35..=3.0).contains(&scale) {
        anyhow::bail!("math scale must be between 0.35 and 3.0 (got {scale})");
    }
    let align = if let Some(align) = cli_align {
        align.into()
    } else if let Some(align) = front.math_block_align.as_deref() {
        parse_math_block_align(align)?
    } else {
        md2any::math::MathBlockAlign::Center
    };
    let max_height = cli_max_height.or(front.math_max_height);
    if let Some(max_height) = max_height {
        if !(24.0..=1200.0).contains(&max_height) {
            anyhow::bail!(
                "math max height must be between 24 and 1200 logical pixels (got {max_height})"
            );
        }
    }
    Ok(md2any::math::MathSvgOptions {
        scale_percent: (scale * 100.0).round() as u16,
        max_height_px: max_height.map(|height| height.round() as u16),
        block_align: align,
    })
}

fn parse_math_block_align(value: &str) -> Result<md2any::math::MathBlockAlign> {
    match value.trim().to_ascii_lowercase().as_str() {
        "left" | "start" => Ok(md2any::math::MathBlockAlign::Left),
        "center" | "centre" | "middle" => Ok(md2any::math::MathBlockAlign::Center),
        "right" | "end" => Ok(md2any::math::MathBlockAlign::Right),
        other => anyhow::bail!("unknown math_block_align: {other} (try left, center, or right)"),
    }
}

fn resolve_document_options(
    cli_style: Option<DocStyleArg>,
    front: &md2any::ir::FrontMatter,
) -> Result<document::DocumentOptions> {
    let style = if let Some(style) = cli_style {
        style.into()
    } else if let Some(style) = front.doc_style.as_deref() {
        parse_doc_style(style)?
    } else {
        document::DocumentStyle::default()
    };
    Ok(document::DocumentOptions::new(style))
}

fn parse_doc_style(value: &str) -> Result<document::DocumentStyle> {
    let normalized = value.trim().to_ascii_lowercase().replace('_', "-");
    match normalized.as_str() {
        "plain" | "simple" => Ok(document::DocumentStyle::Plain),
        "report" => Ok(document::DocumentStyle::Report),
        "handout" => Ok(document::DocumentStyle::Handout),
        "speaker-notes" | "speaker" | "notes" => Ok(document::DocumentStyle::SpeakerNotes),
        other => anyhow::bail!(
            "unknown doc_style: {other} (try plain, report, handout, or speaker-notes)"
        ),
    }
}

fn parse_math_mode(value: &str) -> Result<md2any::math::MathMode> {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "unicode" => Ok(md2any::math::MathMode::Unicode),
        "source" | "raw" | "off" => Ok(md2any::math::MathMode::Source),
        "svg" | "image" | "images" => Ok(md2any::math::MathMode::Svg),
        other => anyhow::bail!("unknown math mode: {other} (try unicode, source, or svg)"),
    }
}

fn report_math_diagnostics(diagnostics: &[md2any::math::MathDiagnostic]) {
    if diagnostics.is_empty() {
        return;
    }
    eprintln!("md2any math check: {} warning(s)", diagnostics.len());
    for diagnostic in diagnostics.iter().take(30) {
        eprintln!(
            "  line {}: [{}] {} — {}",
            diagnostic.line, diagnostic.kind, diagnostic.detail, diagnostic.source
        );
    }
    if diagnostics.len() > 30 {
        eprintln!("  ... {} more", diagnostics.len() - 30);
    }
}

/// Probe the runtime environment and print a human-readable report. Aimed
/// at "why isn't my diagram / CJK / SVG / remote image working" — checks
/// the optional CLIs we shell out to, the optional Cargo features compiled
/// in, and what fonts the binary carries.
fn run_doctor() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    // `say!` returns from the function on BrokenPipe so `md2any doctor |
    // head` doesn't panic. Same pattern as `print_outline`.
    macro_rules! say {
        ($($arg:tt)*) => {
            if writeln!(out, $($arg)*).is_err() { return; }
        };
    }
    macro_rules! blank {
        () => {
            if writeln!(out).is_err() {
                return;
            }
        };
    }

    let tool_present = |name: &str| -> bool {
        let probe = |arg: &str| {
            Command::new(name)
                .arg(arg)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .ok()
                .map(|s| s.success())
                .unwrap_or(false)
        };
        probe("-V") || probe("--version")
    };

    say!("md2any {} — environment check", env!("CARGO_PKG_VERSION"));
    blank!();

    say!("Build features");
    say!(
        "  svg               {}  (SVG image rasterisation via resvg)",
        if cfg!(feature = "svg") { "on " } else { "OFF" }
    );
    say!(
        "  remote-images     {}  (fetching http(s) image URLs)",
        if cfg!(feature = "remote-images") {
            "on "
        } else {
            "OFF"
        }
    );
    blank!();

    say!("Bundled fonts");
    say!(
        "  DejaVu Sans family: {} faces (regular + bold + oblique + bold-oblique + mono)",
        md2any::font::FACE_COUNT
    );
    say!("  CJK fallback:       loaded on demand via `--cjk PATH` (not bundled — too large)");
    say!("  Font licence:       view with `md2any licenses`");
    blank!();

    say!("Optional CLIs for diagram rendering");
    for (name, what) in &[
        ("dot", "Graphviz — ```dot fences"),
        ("mmdc", "Mermaid CLI — ```mermaid fences"),
        ("plantuml", "PlantUML — ```plantuml fences"),
    ] {
        say!(
            "  {:<8}  {}  {}",
            name,
            if tool_present(name) {
                "FOUND  "
            } else {
                "missing"
            },
            what
        );
    }
    blank!();

    say!("Optional CLI for LibreOffice headless verification");
    say!(
        "  libreoffice  {}  (used by `--convert-to pdf` to spot-check ODT/PPTX/DOCX renders)",
        if tool_present("libreoffice") {
            "FOUND  "
        } else {
            "missing"
        }
    );
    blank!();

    if cfg!(feature = "remote-images") {
        say!("Remote-image cache");
        let (dir, platform_ok) = image::remote_cache_status();
        say!("  directory   {}", dir.display());
        if !platform_ok {
            say!(
                "  WARNING    fell back to temp dir (no HOME / XDG_CACHE_HOME / LOCALAPPDATA); \
                 cache won't survive a reboot"
            );
        }
        blank!();
    }

    say!(
        "Working directory: {}",
        std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "?".into())
    );
}

fn run_watch(cli: &Cli, input_path: &Path) -> Result<()> {
    use std::thread::sleep;
    use std::time::Duration;

    let paths: Vec<PathBuf> = cli.inputs.clone();
    let display = paths
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    eprintln!("md2any: watching {} (Ctrl-C to stop)", display);

    let fingerprint = |paths: &[PathBuf]| -> Vec<Option<std::time::SystemTime>> {
        paths
            .iter()
            .map(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
            .collect()
    };

    let mut last = fingerprint(&paths);

    if let Err(e) = build_from_disk(cli, input_path) {
        eprintln!("md2any: error: {e:#}");
    }

    loop {
        sleep(Duration::from_millis(250));
        let now = fingerprint(&paths);
        if now != last {
            last = now;
            sleep(Duration::from_millis(80));
            match build_from_disk(cli, input_path) {
                Ok(()) => {}
                Err(e) => eprintln!("md2any: error: {e:#}"),
            }
        }
    }
}

fn run_serve(cli: &Cli) -> Result<()> {
    let inputs = cli.inputs.clone();
    let watch_paths = inputs.clone();
    let cli_snapshot = serve_cli_snapshot(cli);
    let serve_format: serve::ServeFormat = cli.serve_format.into();
    let build = move || -> Result<serve::ServedArtifact, String> {
        build_artifact_for_serve(&inputs, &cli_snapshot).map_err(|e| format!("{e:#}"))
    };
    let opts = serve::ServeOpts {
        port: cli.port,
        bind: "127.0.0.1".into(),
        edit: cli.edit,
    };
    if cli.edit {
        eprintln!(
            "md2any: editor at http://127.0.0.1:{} — edits save to {}",
            cli.port,
            watch_paths
                .first()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        );
    }
    serve::run(opts, watch_paths, serve_format, build).with_context(|| "preview server")?;
    Ok(())
}

#[derive(Clone)]
struct ServeCli {
    theme: Option<String>,
    aspect: Option<String>,
    layout: Option<String>,
    break_mode: Option<BreakModeArg>,
    break_fill: Option<f32>,
    table_fit: Option<TableFitArg>,
    code_theme: Option<CodeThemeArg>,
    code_columns: Option<CodeColumnsArg>,
    math: Option<MathModeArg>,
    math_scale: Option<f32>,
    math_block_align: Option<MathBlockAlignArg>,
    math_max_height: Option<f32>,
    title: Option<String>,
    author: Option<String>,
    font: Option<String>,
    logo: Option<PathBuf>,
    theme_file: Option<PathBuf>,
    handout: Option<u32>,
    with_notes: bool,
    notes_page_size: NotesPageSizeArg,
    notes_layout: NotesLayoutArg,
    pdf_font: Option<PathBuf>,
    pdf_mono_font: Option<PathBuf>,
    font_fallback: Option<String>,
    cjk: Option<PathBuf>,
    serve_format: ServeFormatArg,
}

fn serve_cli_snapshot(cli: &Cli) -> ServeCli {
    ServeCli {
        theme: cli.theme.clone(),
        aspect: cli.aspect.clone(),
        layout: cli.layout.clone(),
        break_mode: cli.break_mode,
        break_fill: cli.break_fill,
        table_fit: cli.table_fit,
        code_theme: cli.code_theme,
        code_columns: cli.code_columns,
        math: cli.math,
        math_scale: cli.math_scale,
        math_block_align: cli.math_block_align,
        math_max_height: cli.math_max_height,
        title: cli.title.clone(),
        author: cli.author.clone(),
        font: cli.font.clone(),
        logo: cli.logo.clone(),
        theme_file: cli.theme_file.clone(),
        handout: cli.handout,
        with_notes: cli.with_notes,
        notes_page_size: cli.notes_page_size,
        notes_layout: cli.notes_layout,
        pdf_font: cli.pdf_font.clone(),
        pdf_mono_font: cli.pdf_mono_font.clone(),
        font_fallback: cli.font_fallback.clone(),
        cjk: cli.cjk.clone(),
        serve_format: cli.serve_format,
    }
}

fn build_artifact_for_serve(inputs: &[PathBuf], cli: &ServeCli) -> Result<serve::ServedArtifact> {
    if inputs.is_empty() {
        anyhow::bail!("no inputs");
    }
    let first = &inputs[0];
    let first_text =
        std::fs::read_to_string(first).with_context(|| format!("read {}", first.display()))?;
    let input = if inputs.len() == 1 {
        first_text
    } else {
        let mut combined = first_text;
        for p in &inputs[1..] {
            let text =
                std::fs::read_to_string(p).with_context(|| format!("read {}", p.display()))?;
            let (_, body) = front_matter::extract(&text);
            if !combined.ends_with('\n') {
                combined.push('\n');
            }
            combined.push('\n');
            combined.push_str(&body);
        }
        combined
    };

    let (front, body) = front_matter::extract(&input);
    let theme_name = cli
        .theme
        .clone()
        .or_else(|| front.theme.clone())
        .unwrap_or_else(|| "light".into());
    let aspect = cli
        .aspect
        .clone()
        .or_else(|| front.aspect.clone())
        .unwrap_or_else(|| "16:9".into());
    let font_pref = cli.font.clone().or_else(|| front.font.clone());
    let mut theme = theme::Theme::resolve(&theme_name, &aspect, font_pref.as_deref())
        .with_context(|| "resolve theme")?;
    theme.apply_code_theme(resolve_code_theme(cli.code_theme, &front)?);
    let theme_file_dir = cli
        .theme_file
        .as_ref()
        .and_then(|p| p.parent())
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_path_buf());
    if let Some(path) = &cli.theme_file {
        let ov = theme::load_override(path)?;
        theme
            .apply_override(&ov)
            .with_context(|| format!("apply theme overlay {}", path.display()))?;
    }
    let layout_name = cli
        .layout
        .clone()
        .or_else(|| front.layout.clone())
        .unwrap_or_else(|| "clean".into());
    let layout = layout::Layout::resolve(&layout_name).with_context(|| "resolve layout")?;
    let pagination = resolve_pagination_options(
        cli.break_mode,
        cli.break_fill,
        cli.table_fit,
        cli.code_columns,
        &front,
    )?;
    let math_mode = if let Some(mode) = cli.math {
        mode.into()
    } else if let Some(mode) = front.math.as_deref() {
        parse_math_mode(mode)?
    } else {
        md2any::math::MathMode::Unicode
    };
    let math_svg = resolve_math_svg_options(
        cli.math_scale,
        cli.math_block_align,
        cli.math_max_height,
        &front,
    )?;

    let stem_title = first
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Presentation".into());
    let deck_title = cli
        .title
        .clone()
        .or_else(|| front.title.clone())
        .unwrap_or(stem_title);
    let author = cli
        .author
        .clone()
        .or_else(|| front.author.clone())
        .unwrap_or_else(|| "md2any".into());
    let base_dir = first
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    let mut parsed = parser::parse_with_options(
        &body,
        &front,
        &deck_title,
        parser::ParseOptions {
            math_mode,
            math_svg,
            include_base_dir: Some(base_dir.clone()),
            ..Default::default()
        },
    );
    let deck_stem = first
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "deck".into());
    let cache = diagram::cache_dir_for(&deck_stem);
    let _ = diagram::pre_render(&mut parsed, &cache);
    let slides = paginate::paginate_for_layout_with_options(parsed, &theme, &layout, pagination);
    let slides = if front.toc {
        toc::inject(slides)
    } else {
        slides
    };

    let logo_path: Option<PathBuf> = if let Some(p) = &cli.logo {
        if p.exists() || p.is_absolute() {
            Some(p.clone())
        } else {
            Some(base_dir.join(p))
        }
    } else if let Some(p) = &front.logo {
        let p = PathBuf::from(p);
        if p.is_absolute() {
            Some(p)
        } else {
            Some(base_dir.join(p))
        }
    } else {
        None
    };

    let transition = front.transition.as_deref();
    let trans_dur = front.transition_duration.unwrap_or(0.4);
    let direction = front.direction.as_deref();
    let pdf_font_paths = resolve_pdf_font_paths_from_parts(
        cli.pdf_font.as_ref(),
        cli.pdf_mono_font.as_ref(),
        cli.font_fallback.as_deref(),
        cli.cjk.as_ref(),
        &theme,
        theme_file_dir.as_deref(),
    )?;
    match cli.serve_format {
        ServeFormatArg::Pdf => Ok(serve::ServedArtifact::Pdf(pdf::write_with_font_options(
            &slides,
            &theme,
            &layout,
            &deck_title,
            &author,
            &base_dir,
            logo_path.as_deref(),
            cli.handout,
            transition,
            trans_dur,
            direction,
            cli.with_notes,
            cli.notes_page_size.into(),
            cli.notes_layout.into(),
            pdf_font_paths.as_options(),
        )?)),
        ServeFormatArg::Html => Ok(serve::ServedArtifact::Html(html::write(
            &slides,
            &theme,
            &layout,
            &deck_title,
            &author,
            &base_dir,
            logo_path.as_deref(),
            direction,
        )?)),
        ServeFormatArg::Svg | ServeFormatArg::Png => {
            let format = match cli.serve_format {
                ServeFormatArg::Svg => svg::ImageFormat::Svg,
                ServeFormatArg::Png => svg::ImageFormat::Png,
                _ => unreachable!(),
            };
            let files = svg::write_files(
                &slides,
                &theme,
                &layout,
                &deck_title,
                &author,
                &base_dir,
                logo_path.as_deref(),
                direction,
                format,
            )?
            .into_iter()
            .map(|file| serve::ServedFile {
                name: file.name,
                bytes: file.bytes,
            })
            .collect();
            let serve_format: serve::ServeFormat = cli.serve_format.into();
            Ok(serve::ServedArtifact::Images {
                format: serve_format,
                files,
            })
        }
    }
}

/// Dump one line per paginated slide. Intended for debugging pagination
/// behaviour and for pasting into bug reports — nothing here parses, so
/// the format is just `page  kind  N blocks  title  [n][b]`.
fn print_outline(slides: &[md2any::ir::Slide]) {
    use md2any::ir::{Block, SlideKind};
    use std::io::Write;
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let total = slides.len();
    let width = total.to_string().len();
    for (i, s) in slides.iter().enumerate() {
        let kind = match s.kind {
            SlideKind::Title { .. } => "TITLE  ",
            SlideKind::Section => "SECTION",
            SlideKind::Content => "CONTENT",
        };
        let flags = format!(
            "{}{}",
            if s.notes.is_some() { "[notes]" } else { "" },
            if s.bg_image.is_some() { "[bg]" } else { "" },
        );
        let summary = if s.blocks.is_empty() {
            String::from("(empty)")
        } else {
            let counts =
                s.blocks
                    .iter()
                    .fold((0u32, 0u32, 0u32, 0u32, 0u32, 0u32, 0u32), |mut acc, b| {
                        match b {
                            Block::Paragraph(_) => acc.0 += 1,
                            Block::Heading { .. } => acc.1 += 1,
                            Block::List(_) => acc.2 += 1,
                            Block::CodeBlock { .. } => acc.3 += 1,
                            Block::Table { .. } => acc.4 += 1,
                            Block::Image { .. } => acc.5 += 1,
                            _ => acc.6 += 1,
                        }
                        acc
                    });
            let mut parts = Vec::new();
            if counts.0 > 0 {
                parts.push(format!("{}p", counts.0));
            }
            if counts.1 > 0 {
                parts.push(format!("{}h", counts.1));
            }
            if counts.2 > 0 {
                parts.push(format!("{}list", counts.2));
            }
            if counts.3 > 0 {
                parts.push(format!("{}code", counts.3));
            }
            if counts.4 > 0 {
                parts.push(format!("{}tbl", counts.4));
            }
            if counts.5 > 0 {
                parts.push(format!("{}img", counts.5));
            }
            if counts.6 > 0 {
                parts.push(format!("{}other", counts.6));
            }
            parts.join(" ")
        };
        // Swallow BrokenPipe so piping to `head` / `less` doesn't panic.
        if writeln!(
            out,
            "{:>w$}/{}  {}  {:<24}  {}  {}",
            i + 1,
            total,
            kind,
            summary,
            s.title,
            flags,
            w = width,
        )
        .is_err()
        {
            break;
        }
    }
}

fn build_from_disk(cli: &Cli, input_path: &Path) -> Result<()> {
    let start = Instant::now();
    let first = &cli.inputs[0];
    let first_text =
        std::fs::read_to_string(first).with_context(|| format!("read {}", first.display()))?;
    let input = if cli.inputs.len() == 1 {
        first_text
    } else {
        let mut combined = first_text;
        for p in &cli.inputs[1..] {
            let text =
                std::fs::read_to_string(p).with_context(|| format!("read {}", p.display()))?;
            let (_, body) = front_matter::extract(&text);
            if !combined.ends_with('\n') {
                combined.push('\n');
            }
            combined.push('\n');
            combined.push_str(&body);
        }
        combined
    };
    build_once(cli, input_path, &input, None, start)
}
