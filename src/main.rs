//! `md2any` CLI entry point.
//!
//! Parses arguments, resolves theme + layout + aspect, drives the parse →
//! paginate → render → write pipeline, and handles the long tail of
//! convenience flags (`--watch`, `--serve`, `--handout`, `--with-notes`,
//! the `--help-*` self-documenting outputs, etc.). All real work is in
//! the library modules; this file is plumbing.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::time::Instant;

use md2any::{
    diagram, docx, front_matter, image, layout, lint, odp, odt, paginate, parser, pdf, pptx, serve,
    theme, toc,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Pptx,
    Odp,
    Pdf,
    Docx,
    Odt,
}

impl Format {
    fn from_extension(ext: &str) -> Option<Format> {
        match ext.to_lowercase().as_str() {
            "pptx" => Some(Format::Pptx),
            "odp" => Some(Format::Odp),
            "pdf" => Some(Format::Pdf),
            "docx" => Some(Format::Docx),
            "odt" => Some(Format::Odt),
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
            other => anyhow::bail!(
                "unknown format: {} (try pptx, odp, pdf, docx, or odt)",
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
        }
    }
    fn name(&self) -> &'static str {
        match self {
            Format::Pptx => "pptx",
            Format::Odp => "odp",
            Format::Pdf => "pdf",
            Format::Docx => "docx",
            Format::Odt => "odt",
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
                  Five formats supported: pptx, odp, pdf, docx, odt.\n  \
                  pptx/odp/pdf paginate one slide per page.\n  \
                  docx/odt flow continuously (one document, sections page-break on H1).\n\n\
                  Pagination rules (slide formats only):\n  \
                  - First # H1 (or front-matter `title`) → title slide\n  \
                  - Subsequent # H1 → section divider slide\n  \
                  - ## H2 → new content slide\n  \
                  - `---` (horizontal rule) → explicit slide break\n  \
                  - `:::` on its own line → side-by-side columns within a slide\n  \
                  - Lists with > 12 items → auto two-column layout\n  \
                  - `<!-- layout: image-left|image-right -->` → image + text split\n  \
                  - `![alt](src){width=N%}` → resize the image to N% of column\n  \
                  - Long content auto-paginates into '(cont.)' slides\n\n\
                  Front-matter (YAML between `---` fences at top of file):\n  \
                  title, subtitle, author, date, theme, aspect, layout, font, logo,\n  \
                  toc, transition, transition_duration, direction\n\n\
                  CLI flags override front-matter; front-matter overrides defaults.",
    after_help = "Examples:\n  \
                  md2any talk.md                              → talk.pptx\n  \
                  md2any talk.md -o talk.odp                  → ODP\n  \
                  md2any talk.md -o slides.pdf                → PDF\n  \
                  md2any talk.md -o handout.docx              → Word document\n  \
                  md2any talk.md -o notes.odt                 → LibreOffice Writer\n  \
                  md2any talk.md --theme dark --layout studio\n  \
                  md2any talk.md --aspect 9:16                → portrait\n  \
                  md2any talk.md --handout 4 -o handout.pdf   → 4-up A4 handout\n  \
                  md2any talk.md --serve                      → live preview server\n  \
                  md2any --help-pdf --theme dark              → user manual as dark PDF\n  \
                  md2any --help-md > HELP.md                  → dump the manual source\n  \
                  md2any doctor                               → probe optional CLIs + fonts\n  \
                  md2any licenses                             → print bundled-font licence\n\n\
                  Output formats:\n  \
                  pptx  → PowerPoint OOXML; opens in PowerPoint, Keynote, LibreOffice, Slides\n  \
                  odp   → OpenDocument Impress; opens in LibreOffice, Keynote, Slides\n  \
                  pdf   → PDF 1.7; self-contained — embeds DejaVu Sans, optional --cjk\n  \
                  docx  → Microsoft Word; opens in Word, LibreOffice Writer, Pages, Docs\n  \
                  odt   → OpenDocument Writer; opens in LibreOffice, Word, Docs\n\n\
                  Layouts (each composes with light + dark theme + every aspect):\n  \
                  clean   minimal title underline, full-bleed content (default)\n  \
                  studio  editorial — vertical rail, italic titles, big section numerals\n  \
                  frame   sidebar holds deck title and slide number\n  \
                  bold    magazine — full-width accent block behind every title\n\n\
                  Code blocks support 20 languages including: rust, python, js, ts, go,\n  \
                  c, cpp, java, ruby, bash, sql, json, yaml, toml, html, css, cobol, jcl,\n  \
                  rexx, pli, hlasm, db2. Use ` ```rust src/main.rs ` to add a filename caption.\n  \
                  Blocks > 5 lines get line numbers automatically."
)]
struct Cli {
    /// Input markdown file(s) — pass `-` to read from stdin.
    /// Multiple files are concatenated in order; front-matter from the first applies.
    /// Omit when using --help-pptx, --help-odp, --help-pdf, --help-docx, --help-odt, or --help-md.
    #[arg(value_name = "INPUT")]
    inputs: Vec<PathBuf>,

    /// Output file (defaults to input with .pptx extension, or to the extension implied by --format)
    #[arg(short, long, value_name = "PATH")]
    output: Option<PathBuf>,

    /// Output format: pptx | odp | pdf | docx | odt (default: inferred from -o, else pptx)
    #[arg(long, value_name = "NAME")]
    format: Option<String>,

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

    /// Watch the markdown input file(s) and rebuild on change. External
    /// dependencies (images, theme-file, logo) are not tracked — restart
    /// the command if those change. Exits cleanly on Ctrl-C.
    #[arg(long)]
    watch: bool,

    /// Produce a handout PDF with N slides per A4 portrait page (2, 4, or 6).
    /// Only affects PDF output; slide layout/theme are unchanged.
    #[arg(long, value_name = "N")]
    handout: Option<u32>,

    /// Produce a presenter-notes PDF: one A4 portrait page per slide with
    /// the slide thumbnail on top and the speaker notes below. PDF only.
    #[arg(long, conflicts_with = "handout")]
    with_notes: bool,

    /// Path to a CJK font (TTF/TTC/OTF) used as a glyph fallback in PDF
    /// output. Activates whenever a text run contains Chinese/Japanese/
    /// Korean characters that the bundled DejaVu font doesn't cover. Try
    /// `--cjk /usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc`.
    #[arg(long, value_name = "PATH")]
    cjk: Option<PathBuf>,

    /// Start a local HTTP preview server with hot reload. PDF is rebuilt
    /// whenever any input file changes and the browser tab auto-refreshes.
    #[arg(long)]
    serve: bool,

    /// Port for `--serve` (default 8421).
    #[arg(long, value_name = "PORT", default_value_t = 8421)]
    port: u16,

    /// Build the embedded user manual as PPTX (respects --theme, --layout, --aspect, -o)
    #[arg(long, conflicts_with_all = ["help_odp", "help_pdf", "help_md", "help_docx", "help_odt"], help_heading = "Manual")]
    help_pptx: bool,

    /// Build the embedded user manual as ODP (respects --theme, --layout, --aspect, -o)
    #[arg(long, conflicts_with_all = ["help_pptx", "help_pdf", "help_md", "help_docx", "help_odt"], help_heading = "Manual")]
    help_odp: bool,

    /// Build the embedded user manual as PDF (respects --theme, --layout, --aspect, -o)
    #[arg(long, conflicts_with_all = ["help_pptx", "help_odp", "help_md", "help_docx", "help_odt"], help_heading = "Manual")]
    help_pdf: bool,

    /// Build the embedded user manual as Word DOCX (respects --theme, -o)
    #[arg(long, conflicts_with_all = ["help_pptx", "help_odp", "help_pdf", "help_md", "help_odt"], help_heading = "Manual")]
    help_docx: bool,

    /// Build the embedded user manual as LibreOffice ODT (respects --theme, -o)
    #[arg(long, conflicts_with_all = ["help_pptx", "help_odp", "help_pdf", "help_md", "help_docx"], help_heading = "Manual")]
    help_odt: bool,

    /// Print the embedded user-manual Markdown source to stdout
    #[arg(long, conflicts_with_all = ["help_pptx", "help_odp", "help_pdf", "help_docx", "help_odt"], help_heading = "Manual")]
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
    } else {
        None
    };

    let (input_source, input_path): (String, PathBuf) = if let Some(fmt) = help_format {
        let _ = fmt;
        (HELP_MD.to_string(), PathBuf::from("md2any-help.md"))
    } else {
        if cli.inputs.is_empty() {
            anyhow::bail!(
                "no input file (or use --help-pptx, --help-odp, --help-pdf, --help-docx, --help-odt, --help-md, or pipe with '-')"
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
        if help_format.is_some() || cli.check {
            anyhow::bail!("--watch is incompatible with --check / --help-* modes");
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
        if help_format.is_some() || cli.check {
            anyhow::bail!("--serve is incompatible with --check / --help-* modes");
        }
        if cli.inputs.is_empty() {
            anyhow::bail!("--serve requires an input file");
        }
        if cli.inputs.iter().any(|p| p.as_os_str() == "-") {
            anyhow::bail!("--serve cannot be used with stdin input");
        }
        return run_serve(&cli);
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

    let mut parsed = parser::parse(&body, &front, &deck_title);
    // Render diagram code blocks (dot/mermaid/plantuml) to PNGs if the
    // corresponding CLI is available; otherwise leave them as code.
    let deck_stem = input_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "deck".into());
    let cache_dir = diagram::cache_dir_for(&deck_stem);
    let _ = diagram::pre_render(&mut parsed, &cache_dir);
    let slides = paginate::paginate(parsed, &theme);
    let slides = if front.toc {
        toc::inject(slides)
    } else {
        slides
    };

    if cli.check {
        let warnings = lint::check(&slides, &theme);
        lint::report(&warnings);
        std::process::exit(if warnings.is_empty() { 0 } else { 2 });
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

    let output = cli.output.clone().unwrap_or_else(|| {
        if help_format.is_some() {
            PathBuf::from(format!("md2any-help.{}", format.ext()))
        } else {
            let mut p = input_path.to_path_buf();
            p.set_extension(format.ext());
            p
        }
    });

    let n_slides = slides.len();
    let base_dir = input_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));

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
    let bytes = match format {
        Format::Pptx => pptx::write(
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
        )?,
        Format::Odp => odp::write(
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
        )?,
        Format::Pdf => pdf::write(
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
            cli.cjk.as_deref(),
        )?,
        Format::Odt => odt::write(
            &slides,
            &theme,
            &deck_title,
            &author,
            &base_dir,
            logo_path.as_deref(),
            direction,
        )?,
        Format::Docx => docx::write(
            &slides,
            &theme,
            &deck_title,
            &author,
            &base_dir,
            logo_path.as_deref(),
            direction,
        )?,
    };

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
    let build = move || -> Result<Vec<u8>, String> {
        build_pdf_for_serve(&inputs, &cli_snapshot).map_err(|e| format!("{e:#}"))
    };
    let opts = serve::ServeOpts {
        port: cli.port,
        bind: "127.0.0.1".into(),
    };
    serve::run(opts, watch_paths, build).with_context(|| "preview server")?;
    Ok(())
}

#[derive(Clone)]
struct ServeCli {
    theme: Option<String>,
    aspect: Option<String>,
    layout: Option<String>,
    title: Option<String>,
    author: Option<String>,
    font: Option<String>,
    logo: Option<PathBuf>,
    theme_file: Option<PathBuf>,
    handout: Option<u32>,
    with_notes: bool,
    cjk: Option<PathBuf>,
}

fn serve_cli_snapshot(cli: &Cli) -> ServeCli {
    ServeCli {
        theme: cli.theme.clone(),
        aspect: cli.aspect.clone(),
        layout: cli.layout.clone(),
        title: cli.title.clone(),
        author: cli.author.clone(),
        font: cli.font.clone(),
        logo: cli.logo.clone(),
        theme_file: cli.theme_file.clone(),
        handout: cli.handout,
        with_notes: cli.with_notes,
        cjk: cli.cjk.clone(),
    }
}

fn build_pdf_for_serve(inputs: &[PathBuf], cli: &ServeCli) -> Result<Vec<u8>> {
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

    let mut parsed = parser::parse(&body, &front, &deck_title);
    let deck_stem = first
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "deck".into());
    let cache = diagram::cache_dir_for(&deck_stem);
    let _ = diagram::pre_render(&mut parsed, &cache);
    let slides = paginate::paginate(parsed, &theme);
    let slides = if front.toc {
        toc::inject(slides)
    } else {
        slides
    };

    let base_dir = first
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
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
    pdf::write(
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
        cli.cjk.as_deref(),
    )
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
