# Changelog

All notable changes to md2any will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **In-browser editor** (`--serve --edit`): edit the markdown and see a live
  preview side by side. Edits autosave to the source file, which the existing
  watcher rebuilds — no external dependencies, fully offline. The preview is
  true **live-DOM editing**: the deck renders as HTML and each rebuild is
  *morphed* into the existing DOM, so only the changed slide's nodes update
  while every other slide keeps its scroll position and loaded images — no
  flash, no jump to the top. The slide under the caret is highlighted and
  scrolled into view as you type, mapped exactly via per-slide source lines
  (`data-line`). Defaults to the HTML preview; `--serve-format pdf|svg|png`
  still work in the editor but reload per rebuild rather than morphing.
  Clicking a slide in the preview moves the source caret to the line that
  produced it (two-way binding), and a status read-out shows the current
  slide. Pending edits are flushed on tab close so nothing is lost, and a
  build error keeps the last good preview on screen instead of clobbering it.
- **Editor style panel**: a slide-in drawer (🎨 Style) with theme swatches,
  aspect/transition toggles, colour pickers (accent/background/title/text),
  title/body size sliders, and a font-family field. Controls read and write
  the document's front-matter, so choices persist in the file and flow through
  the normal live-preview path.
- **Editor "Generate ▾" menu**: export the deck you're editing straight to
  PPTX, ODP, PDF, DOCX, ODT, or HTML (`GET /export?format=…`), or download the
  current markdown — pending edits are flushed first so the export is current.
- **Editor AI dock** (🤖 Assistant): a collapsible chat panel that can see the
  live document and knows md2any's markup. Ask questions or request edits; when
  it proposes changes it returns the full updated document, which you apply with
  one click — the change autosaves and morphs into the preview. Replies
  **stream** into the dock token-by-token (the `POST /chat` route relays the
  provider's SSE as newline-delimited JSON). Quick-action
  chips cover common asks (proofread, tighten, add a summary slide, speaker
  notes, retitle). Speaks the OpenAI-compatible chat API via a `POST /chat`
  route; provider-neutral. The key comes from `$MD2ANY_API_KEY` /
  `$OPENAI_API_KEY`, or a gitignored key file that also selects the provider —
  drop `grok-api.key` to use xAI/Grok, `md2any-openai-api.key` for OpenAI, with
  the right endpoint/model chosen automatically (overridable with
  `--ai-endpoint`/`--ai-model`). Needs the default `ai` feature.
- **Inline `style:` front-matter**: a `ThemeOverride` block (colours, fonts,
  sizes, title alignment, layout geometry) layered over the named `theme`,
  equivalent to a `--theme-file` but authored in the document. Honoured by all
  renderers; written by the editor's style panel.
- **Theme gallery**: six new built-in themes alongside `light`/`dark` —
  `corporate`, `sepia`, `contrast`, `midnight`, `terminal`, `pastel` — selectable
  with `--theme NAME`. Each is a palette over a light/dark base, so all aspects,
  layouts, and `--theme-file` overrides still apply. Code-block colours now track
  the theme's background luminance automatically.
- **Per-theme title alignment** (`title_align: left|center` in a theme-file, used
  by the `corporate`/`contrast` themes), honoured in PDF/SVG/PNG/HTML.
- **`--list-themes`** prints the built-in theme names; new theming reference at
  `docs/theming.md`.
- **Customisable layout geometry** (v1 of the custom-layout system): a `layout:`
  block in a `--theme-file` overrides a base layout's knobs — `rail_width`,
  `sidebar_width`, `title_underline`, `section_full_bg` — layered on the chosen
  `--layout`. `Layout` is now data-driven for these knobs rather than hardcoded
  per kind.
- **AI deck drafting** (`--generate "prompt"`): draft a deck with a chat model,
  then render it through the normal pipeline. Speaks the OpenAI-compatible
  `/v1/chat/completions` API; endpoint, model, and key (`$MD2ANY_API_KEY` /
  `$OPENAI_API_KEY`) are configurable, and `--save-md` keeps the markdown. On
  by default via the `ai` feature; drop it for a network-free build.

### Fixed

- **HTML code blocks rendered empty.** The HTML renderer emitted theme font
  sizes (stored in centipoints, e.g. `1500` = 15pt) straight into CSS as
  `1500pt`. Headings and body text were masked by their `clamp()` upper bound,
  but `pre` used the raw value — so code was set at ~2000px per glyph and
  showed as an empty box. Sizes are now converted to points and code scales
  with a `clamp()` like body text.

## [0.3.0] — 2026-06-14

A math-rendering and robustness release. The native math layout engine gained
font-aware metrics, real bold, and properly-centred accents; custom OTF/CFF
fonts now embed correctly in PDF; and a broad rendering sweep fixed several
overflow / silent-drop bugs.

### Added

- **Font-aware math metrics.** The math layout engine now measures glyph
  advances against the face that will actually render them (the embedded PDF
  font, or bundled DejaVu for SVG/PNG) via a `GlyphMetrics` provider, so
  reserved width matches drawn glyphs — equations stay aligned under
  `--pdf-font`, not just with the default font.
- **Math bold.** `\mathbf`, `\textbf`, `\boldsymbol`, `\bm`, `\pmb`, `\mathbfit`
  now render in real bold weight (PDF/SVG/PNG); previously a no-op, and
  `\boldsymbol` leaked literal source.
- **`\dfrac` / `\tfrac` / `\cfrac`** and **`\mid` / `\vert` / `\Vert`** are now
  recognised (previously leaked literal source).
- **GFM table column alignment** (`:---`, `:---:`, `---:`) is honoured in PDF,
  SVG, PNG, and HTML.
- **Inline `<svg>…</svg>`** blocks are now rasterised and embedded instead of
  being silently dropped.

### Fixed

- **OTF/CFF font embedding.** PDF now emits `CIDFontType0` + `FontFile3` for
  CFF/OpenType faces (e.g. STIX Two Math) instead of mislabelling them as
  TrueType, which strict readers rejected.
- **Math accents** (`\bar`, `\hat`, `\vec`, `\tilde`, `\dot`, `\ddot`) are drawn
  as centred geometry rather than zero-advance combining marks, which had
  landed at the glyph's right edge (and differently per font — e.g. `\bar{d}`
  rendered as "đ").
- **Long titles no longer overflow.** Title slides (all layouts) and section
  dividers auto-shrink to fit; wrapped content-slide headings push the body and
  underline down instead of overprinting them.
- **Nested blockquotes** (`>`/`>>`/`>>>`) keep every level instead of rendering
  only the deepest.
- **Long unbreakable tokens** (URLs, hashes, CamelCase) hard-break to wrap
  instead of overflowing the slide edge (PDF/SVG); HTML uses `overflow-wrap`.
- **Tabs in code** expand to 4-column tab stops instead of rendering as
  missing-glyph boxes.
- **`{width=N%}` > 100** clamps instead of leaking the literal attribute text.
- **`:::` columns** handle leading/trailing/duplicate dividers (the fence form
  `::: … ::: … :::`) instead of dumping all content into the right column.
- **Task-list items** show only their checkbox, not a checkbox *and* a bullet
  (PDF/SVG/HTML/PPTX).
- **Display-math pagination** weights generated equation images by their real
  aspect ratio, so a one-line equation no longer splits onto a near-empty
  continuation slide.

### Known limitations

- Right-to-left text uses `direction: rtl` for right-alignment but does not
  perform full Unicode bidi reordering of mixed LTR/RTL runs.
- CJK and colour-emoji glyphs require `--cjk <font>` for PDF (the bundled DejaVu
  face does not cover them).
- Table column alignment is not yet applied to the editable Office/ODF outputs
  (pptx/odp/odt/docx).

## [0.2.0] — 2026-05-24

A feature-and-fix-heavy follow-up to the initial release. New CLI surface
for non-Latin scripts, presenter workflows, debugging; richer per-slide
control over layout and images; and a remote-image cache with retry,
placeholders, and stress testing.

### Added

- **`--cjk <PATH>`** — embed a CJK font (TTF / TTC / OTF) in PDF output
  as a per-character fallback for codepoints DejaVu doesn't cover. Subsets
  to just the glyphs the deck uses so a 20 MB Noto CJK source typically
  contributes only kilobytes to the output PDF.
- **`--with-notes`** — produce a presenter-notes PDF: one A4 page per
  slide with the slide thumbnail on top and the speaker notes below.
  Mutually exclusive with `--handout`.
- **`--outline`** — parse + paginate, then dump a one-line-per-slide
  outline (page, kind, block summary, notes/bg flags) and exit. Useful
  for debugging pagination and pasting into bug reports.
- **`md2any doctor`** — probe the environment: optional CLIs (`dot`,
  `mmdc`, `plantuml`, `libreoffice`), build feature flags, bundled font
  count, resolved remote-image cache directory.
- **`md2any licenses`** — print the embedded font licence notice. The
  DejaVu / Bitstream Vera / Arev licences require the notice to travel
  with the font programs; this command surfaces it for binary-only
  distributions.
- **Per-slide layout directives** — `<!-- layout: image-left -->` and
  `<!-- layout: image-right -->` split a slide with one image + text into
  a two-column layout, reusing the existing `:::` column renderer.
- **Image sizing** — Pandoc-style `![alt](src){width=N%}` attribute
  resizes the image to N% of the column. Honoured by PDF, PPTX, and ODP
  slide renderers.
- **Remote image cache** — http(s) image URLs fetched at build time are
  cached under the platform cache directory and reused on subsequent
  renders. Controls: `--remote-image-cache <PATH>`,
  `--no-remote-image-cache`, `--remote-image-user-agent <STRING>`.
- **Retry on transient HTTP failures** — 408 / 429 / 500 / 502 / 503 /
  504 and network errors are retried up to three times with capped
  exponential backoff (max 30 s), honouring `Retry-After` in both
  delta-seconds and HTTP-date forms.
- **Placeholder substitution on image failure** — any image load failure
  (network, 404, garbage, empty body, payload over the 20 MB cap, broken
  SVG, missing local file) substitutes a red "Image failed to load"
  placeholder containing the URL and error, prints a one-line stderr
  warning, and the render completes. CI pipelines can grep stderr for
  `warning: image failed` to surface the issue without failing the build.
- **PDF font subsetting** — every PDF embeds only the glyphs the deck
  actually references, not the full 3 MB face. A typical talk-sized deck
  lands under 200 KB even with code, tables, and the full Greek / math
  toolbox.
- **SVG image support** — `![alt](path.svg)` rasterises via `resvg` and
  `tiny-skia` at 192 DPI (2× retina) using the bundled DejaVu fonts so
  text renders identically regardless of the build machine's installed
  fonts. Gated on the `svg` feature (default on).
- **Three new example decks** under `examples/`:
  - `special-relativity.md` — math, history, tables, quotes, three
    hand-authored SVG diagrams (CC0)
  - `sorting-algorithms.md` — code-highlighting torture test across
    Python, Rust, COBOL, JCL, PL/I; complexity tables
  - `periodic-table.md` — 14 tables, scientific notation, Unicode
    superscripts / subscripts / element symbols
- **Demo bundle** — `examples/demo.{pptx,odp,pdf,docx,odt}` regenerated
  and committed so new users can inspect output without installing.
- **Integration stress test** — `tests/cache_stress.rs` (gated behind
  `cargo test -- --ignored`) drives 24 assertions against a local HTTP
  server covering cold fetch, warm cache, URL normalisation, cache
  disable, retry on 503, oversize cap, garbage / empty / concurrent
  scenarios. Skips cleanly when Python 3 / curl / the compiled binary
  are missing.
- **10 image unit tests + 4 parser regression tests** for `fnv1a64`,
  URL normalisation, `Retry-After` parsing (both forms), retry-delay cap,
  and `extract_image_attrs` UTF-8 handling.

### Changed

- **`HELP.md` rewritten and expanded** to 136 slides. New sections:
  per-construct math reference (12 slides), full `--theme-file` reference
  with colour / font / syntax tables, compatibility matrix across viewers,
  known limitations with "not Pandoc / not Quarto" positioning, bundled
  font licence pointer, showcase section exercising every feature.
- **Pagination algorithm overhaul**:
  - Width-aware line-wrap estimates — portrait aspects no longer
    overestimate body line capacity.
  - Chrome-aware budget — title bar + footer subtracted before scaling so
    A4 portrait and 9:16 don't inflate the per-line allowance.
  - Code-block per-line weight (0.7) — long flag listings no longer split
    across near-empty pages.
  - Table chunking — long reference tables auto-split with the header
    row repeated on each continuation slide.
  - Orphan-lead carry — single lead-in paragraphs that would otherwise
    sit alone get pulled forward to the next page.
- **CI** — `cargo fmt --check` is now a required job (was advisory);
  clippy stays advisory for v0.2.0.
- **Theme overlay validation** — `--theme-file` rejects non-hex colour
  values with a clear error rather than corrupting downstream XML.

### Fixed

- **UTF-8 mojibake in body paragraphs.** A byte-cast in
  `extract_image_attrs` turned every multi-byte UTF-8 character (Greek
  letters, em-dashes, math operator output) into Latin-1 garbage. The
  manual's Math reference tables and `Renders as:` paragraphs were
  affected. Regression test added.
- **Title-slide subtitle overlap.** When the title wrapped to a second
  line, the subtitle / author block was positioned assuming a one-line
  title and rendered through the wrapped text. Fixed in all four layouts
  (Clean / Studio / Frame / Bold).
- **Ordered-list bullet overlap.** Items numbered 10 and above
  overflowed the fixed 0.33" bullet gutter, so the period rendered
  beneath the first letter of the body text. Gutter now measured per item.
- **Image weight underestimate.** `Block::Image` weighted at 6.0 was
  letting text-after-image slides overflow the page footer; bumped to
  13.0 to match the renderer's 65% slide-height cap.
- **Background propagation through `(cont.)` slides** — when pagination
  split a slide with `<!-- bg: ... -->`, the continuation pages lost the
  background.
- **PlantUML diagrams silently producing missing-file references** —
  `plantuml -pipe` writes to stdout (the `-o` flag is ignored in pipe
  mode); we were discarding stdout. Now captured to the output path.
- **DOCX + ODT mixed-list rendering.** A list mixing ordered and
  unordered items was coerced to one style for the whole block. DOCX now
  picks `numId` per item; ODT closes and re-opens the `<text:list>`
  wrapper when the ordered flag flips, since ODF carries the style on
  the wrapper.
- **`(cont.)` title suffix on the first emitted page** — orphan-carry
  could swallow the initial split attempt; the page that finally shipped
  was still page 1 of the slide and shouldn't carry the suffix.
- **Section-slide `kicker_y` underflow** — `h - 2_600_000` for slides
  taller than 2.7 in is fine; for shorter custom aspects it would
  underflow `u32` and panic. Now uses `saturating_sub`.
- **Remote-image cache hygiene** — garbage, empty, or oversize responses
  no longer get written to the cache. `sniff()` runs before the cache
  write, so a 200 OK with HTML body or zero bytes is rejected up front.
- **Remote-image truncation undetected** — the 20 MB cap was a silent
  truncation; oversize payloads now error loudly (or fall through to the
  placeholder).
- **`--watch` doc string** — claimed it watched referenced images, which
  it doesn't. Trimmed to match reality.

### Internal

- Constants extracted: `theme::IMAGE_MAX_HEIGHT_FRACTION_*` (shared by
  PDF / PPTX / ODP renderers + pagination weight),
  `theme::LONG_LIST_THRESHOLD` (shared across 6 sites).
- Dead code removed: `pdf::rewrite_content_glyphs`,
  `pdf::build_tounicode_cmap_remapped`, `odt::_injection_anchor`.
  ~98 lines deleted across the cleanup pass.
- Module docs updated: `image.rs`, `paginate.rs`, `layout.rs`, `main.rs`
  all have module-level `//!` headers; `font.rs` and `image.rs` headers
  rewritten to reflect their current breadth.
- All test totals: 14 image unit + 14 renderer + 4 snapshot + 1 doctest
  + 1 ignored integration test, all pass.

## [0.1.0] — 2026-05-23

Initial release.

### Output formats

- **PowerPoint** (`.pptx`) — native OOXML, editable in PowerPoint, Keynote,
  LibreOffice Impress, Google Slides.
- **OpenDocument Impress** (`.odp`) — native ODF, smaller than PPTX, editable
  in LibreOffice Impress, Keynote, Slides.
- **PDF** (`.pdf`) — PDF 1.7, pure Rust, no external library. Embeds the
  bundled DejaVu Sans family (regular / bold / oblique / bold-oblique /
  mono) for broad Unicode coverage including Greek, math operators,
  sub/superscripts.
- **Microsoft Word** (`.docx`) — native OOXML document, flowing text with
  proper heading / list / table / image / hyperlink markup.
- **LibreOffice Writer** (`.odt`) — native ODF document.

### Themes & layouts

- Two themes: `light` and `dark`.
- Four layouts: `clean`, `studio`, `frame`, `bold`.
- Custom theme files via `--theme-file <path>.yaml` — override colours,
  fonts, and sizes on top of the base theme.

### Aspect ratios

- Presets: `16:9`, `4:3`, `9:16`, `a3`, `a4`, `a4-landscape`, `a5`,
  `letter`, `letter-landscape`, `legal`, `tabloid`.
- Custom: `WxH[unit]` with `px` (default at 96 DPI), `mm`, `cm`, `in`, `pt`,
  `emu`.

### Markdown features

- Paragraphs, headings (H1–H6), bulleted + numbered nested lists, tables,
  block quotes, fenced code blocks, inline `code` / **bold** / *italic* /
  ~~strikethrough~~ / [links].
- Side-by-side columns via `:::` separator.
- GitHub-flavoured tables.
- Syntax highlighting for 20 languages including Rust, Python, JavaScript,
  TypeScript, Go, C/C++, Java, Ruby, Bash, SQL, JSON, YAML, TOML, HTML,
  XML, CSS, and mainframe languages (COBOL, JCL, REXX, PL/I, HLASM, DB2).
- LaTeX-flavoured math (`$inline$`, `$$display$$`) → Unicode at parse time.
- Diagrams: ` ```dot ` / ` ```mermaid ` / ` ```plantuml ` shell out to the
  matching CLI tool if installed.
- Footnotes (`[^id]`) with definitions collected at slide bottom.
- Speaker notes via `<!-- notes: -->` HTML comments.
- Per-slide background image via `<!-- bg: path -->`.
- Auto Table-of-Contents slide via `toc: true` in front-matter.

### Workflow

- `--watch`: rebuild on file change.
- `--serve [--port N]`: localhost HTTP preview with hot reload (no Chromium).
- `--check`: lint mode — exit code 2 on warnings, no file written.
- `md2any new <path>`: scaffold a starter deck.
- Multi-file input: concatenate several markdown files; first one's
  front-matter wins.
- Stdin input via `-`.

### Output controls

- `--handout 2|4|6`: N-up A4 portrait PDF for printing.
- `--logo <path>`: render a footer logo on every content slide.
- Slide transitions: `fade`, `push`, `wipe`, `cover`, `split` (PPTX, ODP,
  PDF).
- RTL support via `direction: rtl` (PPTX, ODP).
- Clickable hyperlinks in PDF, PPTX, ODP, DOCX, ODT.
- Self-documenting: `--help-md`, `--help-pptx`, `--help-odp`, `--help-pdf`,
  `--help-docx`, `--help-odt`.

### Library

- Public Rust crate. See `docs.rs/md2any` for the API.
- All renderers callable independently of the CLI.
- Snapshot tests over the parser → paginator pipeline.

### Performance

- 30-slide deck → PPTX in ~1 ms, PDF in ~5 ms on commodity x86-64.
- 100-slide deck → PPTX in ~3 ms, PDF in ~12 ms.

[Unreleased]: https://github.com/javaperformance/md2any/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/javaperformance/md2any/releases/tag/v0.1.0
