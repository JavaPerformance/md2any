# md2any

[![CI](https://github.com/javaperformance/md2any/actions/workflows/ci.yml/badge.svg)](https://github.com/javaperformance/md2any/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/md2any.svg)](https://crates.io/crates/md2any)
[![docs.rs](https://docs.rs/md2any/badge.svg)](https://docs.rs/md2any)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

**One markdown source → five document formats.** A single self-contained Rust
binary that converts markdown into editable PowerPoint, OpenDocument Impress,
PDF, Word, and LibreOffice Writer files.

```bash
md2any talk.md                    # → talk.pptx     (PowerPoint)
md2any talk.md -o talk.odp        # → talk.odp      (LibreOffice Impress)
md2any talk.md -o talk.pdf        # → talk.pdf      (PDF 1.7)
md2any talk.md -o talk.docx       # → talk.docx     (Microsoft Word)
md2any talk.md -o talk.odt        # → talk.odt      (LibreOffice Writer)
```

No Office install. No headless Chromium. No LaTeX. No Node. No Python. One ~5 MB binary.

## Why md2any

| | md2any | [Pandoc] | [Marp CLI] | [Slidev] | [Quarto] |
|---|:---:|:---:|:---:|:---:|:---:|
| Editable PPTX | ✅ | ✅ | ❌ *(image-baked)* | ❌ | ✅ |
| Native ODP / ODT | ✅ | ✅ | ❌ | ❌ | ✅ |
| Native PDF (no LaTeX) | ✅ | ❌ | ❌ *(needs Chromium)* | ❌ *(needs Chromium)* | ❌ |
| DOCX | ✅ | ✅ | ❌ | ❌ | ✅ |
| Single self-contained binary | ✅ | ✅ | ⚠️ | ❌ | ❌ |
| Install size | ~5 MB | ~100 MB | ~150 MB | ~250 MB | ~500 MB+ |
| Cold start | ~5 ms | ~100 ms | ~2 s | ~3 s | ~3 s |
| Live preview server | ✅ | ❌ | ✅ | ✅ | ✅ |

[Pandoc]: https://pandoc.org/
[Marp CLI]: https://github.com/marp-team/marp-cli
[Slidev]: https://sli.dev/
[Quarto]: https://quarto.org/

md2any is the only tool in this list that produces every common Office /
OpenDocument format **natively** from a single small binary — no headless
browser, no LaTeX stack, no language runtime.

## Install

### Prebuilt binaries

Grab the latest release for your platform from
[GitHub Releases](https://github.com/javaperformance/md2any/releases/latest).

- **macOS (Apple Silicon)**: `md2any-vX.Y.Z-aarch64-apple-darwin.tar.gz`
- **macOS (Intel)**: `md2any-vX.Y.Z-x86_64-apple-darwin.tar.gz`
- **Linux x86_64**: `md2any-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz`
- **Linux ARM64**: `md2any-vX.Y.Z-aarch64-unknown-linux-gnu.tar.gz`
- **Windows x86_64**: `md2any-vX.Y.Z-x86_64-pc-windows-msvc.zip`

### From crates.io

```bash
cargo install md2any
```

Requires Rust 1.74+. Builds natively on Linux, macOS, and Windows.

### From source

```bash
git clone https://github.com/javaperformance/md2any
cd md2any
cargo build --release
./target/release/md2any --help
```

For cross-compilation targets, the test suite, the opt-in integration
tests, and the release workflow, see [CONTRIBUTING.md](CONTRIBUTING.md).

## Hello deck

`hello.md`:

```markdown
---
title: My Talk
author: Your Name
date: auto
theme: dark
---

# Welcome

A subtitle for the title slide.

# The pitch

- Markdown in
- Slides out
- One binary

# The features

| Feature | Status |
|---------|--------|
| Five output formats | ✅ |
| Syntax-highlighted code | ✅ |
| Math, diagrams, footnotes | ✅ |

# Code

```rust
fn main() {
    println!("hello, md2any");
}
```

# Q & A
```

Then:

```bash
md2any hello.md                 # → hello.pptx
md2any hello.md -o hello.pdf    # → hello.pdf
md2any hello.md --serve         # live preview at http://localhost:8421
```

## Features

| Category | Highlights |
|----------|------------|
| **Output** | pptx · odp · pdf · docx · odt — all native, no external converters |
| **Themes** | Light + dark built-in, full custom palettes via YAML overlay |
| **Layouts** | Clean / studio / frame / bold |
| **Aspect ratios** | 16:9, 4:3, 9:16, A3/A4/A5, Letter, Legal, Tabloid + landscape variants + custom `WxH[unit]` |
| **Markdown** | Lists (9 deep nesting), tables, code blocks (20 languages incl. mainframe), block quotes, footnotes, links (clickable), strike, inline code |
| **Layout extras** | Side-by-side columns (`:::`), per-slide background (`<!-- bg: -->`), TOC injection, hand-tuned pagination with widow/orphan control |
| **Math** | `$inline$` and `$$display$$` LaTeX subset → Unicode |
| **Diagrams** | Embedded `dot`, `mermaid`, `plantuml` (shells out if installed) |
| **Speaker notes** | `<!-- notes: -->` HTML comments |
| **Transitions** | `fade`, `push`, `wipe`, `cover`, `split` (PPTX/ODP/PDF) |
| **RTL / CJK** | `direction: rtl` for Arabic/Hebrew; CJK via the consumer's font |
| **Workflow** | `--watch` for rebuild-on-save, `--serve` for live preview with hot reload, `--check` for lint mode |
| **Handouts** | `--handout 2|4|6` for N-up A4 printable PDF |
| **Multi-file** | Concat multiple `.md` files; `-` reads stdin |
| **Images** | Local PNG/JPEG/SVG plus cached remote `http(s)` image URLs |
| **Logo** | `--logo brand.png` renders in every slide footer |

Full reference: `md2any --help-md` writes the embedded user manual to stdout, or
`md2any --help-pdf` produces it as a dark-theme PDF.

## Quickstart cheat-sheet

```text
md2any <INPUT...> [OPTIONS]
md2any new <PATH>                           Write a starter deck

  -o, --output <PATH>      Output file
      --format <NAME>      pptx | odp | pdf | docx | odt
      --theme <NAME>       light | dark
      --aspect <RATIO>     preset (16:9, 4:3, 9:16, a4[-landscape], a3, a5,
                           letter[-landscape], legal, tabloid) or custom WxH[unit]
      --layout <NAME>      clean | studio | frame | bold
      --theme-file <PATH>  YAML colour/font overrides
      --logo <PATH>        Footer logo image
      --remote-image-cache <PATH>
                           Cache directory for remote image downloads
      --no-remote-image-cache
                           Fetch remote images every time
      --handout <N>        2/4/6 slides per A4 portrait page (PDF only)
      --watch              Rebuild on file change
      --serve [--port N]   Live preview HTTP server
      --check              Lint mode (warnings → exit 2)
      --help-{pptx,odp,pdf,docx,odt,md}     User manual in any format
```

## Pagination rules

- First `# H1` (or front-matter `title`) → **title slide**
- Subsequent `# H1` → **section divider**
- `## H2` → **content slide**
- `---` (horizontal rule) → explicit slide break
- `:::` on its own line → side-by-side columns
- Lists with > 12 items → auto two-column layout
- Long content auto-paginates into `(cont.)` slides

For DOCX/ODT output, pagination is replaced with flowing text — H1 becomes a
page-break heading, H2 stays inline, the rest flows continuously.

## Front-matter

```yaml
---
title: My Talk
subtitle: A subtitle for the title slide
author: Your Name
date: auto              # or 2026-05-23, or "today"
theme: light            # light | dark
aspect: 16:9            # see aspect ratios above
layout: clean           # clean | studio | frame | bold
font: Inter             # any installed font (PPTX/ODP/DOCX/ODT)
logo: brand.png         # rendered in slide footer
toc: true               # inject a Contents slide after the title
transition: fade        # none | fade | push | wipe | cover | split
transition_duration: 0.6
direction: ltr          # ltr | rtl
---
```

Every key is optional. CLI flags override front-matter; front-matter overrides
defaults.

## Aspect ratios

| Flag value       | Dimensions (mm)    | Dimensions (in)   | Use for                       |
|------------------|--------------------|-------------------|-------------------------------|
| `16:9` (default) | 338.7 × 190.5 mm   | 13.33″ × 7.5″     | Modern projectors and screens |
| `4:3`            | 254 × 190.5 mm     | 10″ × 7.5″        | Legacy projectors             |
| `9:16`           | 190.5 × 338.7 mm   | 7.5″ × 13.33″     | Vertical / phone-shaped       |
| `a4` / `a4-landscape` | 210 × 297 mm  | 8.27″ × 11.69″    | A4 portrait/landscape (ISO 216) |
| `a3` / `a5`      | 297 × 420 / 148 × 210 mm | — | Larger / smaller ISO sizes |
| `letter` / `letter-landscape` | 215.9 × 279.4 mm | 8.5″ × 11″ | US Letter portrait/landscape |
| `legal`          | 215.9 × 355.6 mm   | 8.5″ × 14″        | US Legal                      |
| `tabloid`        | 279.4 × 431.8 mm   | 11″ × 17″         | Tabloid / ANSI B              |
| **custom**       | `WxH[unit]`        | px (default), mm, cm, in, pt, emu | Anything else, e.g. `1920x1080`, `300x200mm`, `13.3x7.5in` |

## Performance

| Deck size  | PPTX  | ODP   | DOCX  | ODT   | PDF    |
|------------|-------|-------|-------|-------|--------|
| 30 slides  | ~1 ms | ~1 ms | ~1 ms | ~1 ms | ~5 ms  |
| 100 slides | ~3 ms | ~2 ms | ~2 ms | ~2 ms | ~12 ms |
| 1000 slides | ~30 ms | ~25 ms | ~20 ms | ~20 ms | ~120 ms |

Numbers from a commodity x86-64 machine. PDF is slower because of PNG decode
and per-token text positioning.

## Library use

The renderer is also a regular Rust crate. See [docs.rs/md2any] for the public
API. Minimal embed:

```rust
let md = std::fs::read_to_string("talk.md")?;
let (front, body) = md2any::front_matter::extract(&md);
let theme = md2any::theme::Theme::resolve("light", "16:9", None)?;
let layout = md2any::layout::Layout::resolve("clean")?;
let slides = md2any::parser::parse(&body, &front, "talk");
let slides = md2any::paginate::paginate(slides, &theme);
let bytes = md2any::pdf::write(
    &slides, &theme, &layout, "talk", "Author",
    std::path::Path::new("."), None, None, None, 0.4, None,
)?;
std::fs::write("talk.pdf", bytes)?;
```

[docs.rs/md2any]: https://docs.rs/md2any

## Limitations

- PNG, JPEG, and SVG images are supported; GIF and WebP are not.
- Remote `http(s)` images are supported when the default `remote-images`
  feature is enabled, with successful downloads cached between renders.
- PDF uses DejaVu Sans + DejaVu Sans Mono embedded in the binary; no other
  font families ship — `--font` only affects PPTX/ODP/DOCX/ODT
- DejaVu does not include CJK glyphs; for PDF Chinese/Japanese/Korean
  decks, export PPTX with `--font "Noto Sans CJK SC"` and convert via the
  viewer
- Speaker notes appear in PPTX / ODP only

See [HELP.md](HELP.md) for the embedded user manual, or run
`md2any --help-md` to print it.

## Contributing

Bug reports, feature suggestions, and pull requests welcome. See
[CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## License

Dual-licensed under either of:

- MIT License ([LICENSE-MIT](LICENSE-MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))

at your option.

Bundled DejaVu font files are redistributed under their own permissive font
license; see [assets/fonts/LICENSE.md](assets/fonts/LICENSE.md). Release
archives that include the standalone binary should include that notice because
the font programs are embedded in the executable.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in md2any by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
