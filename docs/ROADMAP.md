# md2any Roadmap

This roadmap captures the larger product direction for md2any beyond the
current native PPTX/ODP/PDF/DOCX/ODT renderer. The bias is to keep the core
promise intact: one small native binary, editable Office/OpenDocument outputs,
no browser or TeX runtime by default, and explicit CLI/API controls for behavior
that users need to tune.

## Principles

- Keep the default binary small. Large assets and heavy renderers should be
  opt-in through user-supplied paths, optional features, or external tools.
- Keep outputs native and editable where the target format supports it.
- Add debuggability before adding more complexity: users should be able to see
  why a deck rendered the way it did.
- Update `--help`, embedded `HELP.md`, README, and library examples together
  whenever public behavior changes.
- Prefer command-line and API controls over environment variables.

## Phase 1: Layout Correctness and Debuggability

- Add a shared render-plan layer between `ir`/`paginate` and output writers.
  It should record measured blocks, resolved positions, continuations, source
  line references, asset choices, font choices, and warnings.
- Add `--emit-ir PATH`, `--emit-plan PATH`, and `--trace-layout` for stable JSON
  diagnostics. These should be useful in CI and in bug reports.
- Extend renderer tests to diff render plans before byte-level output details.
- Continue reducing duplicated measurement logic across PPTX, ODP, and PDF.

## Phase 2: Presenter and Speaker Workflows

- Make PDF notes pages use the deck page size by default so widescreen decks do
  not waste space on A4 portrait pages.
- Keep A4 notes pages available for print workflows with an explicit switch.
- Add a speaker package command that can emit the deck, notes PDF, handout PDF,
  and an asset manifest in one command.
- Consider `--notes-layout auto|below|side-by-side` after the render-plan layer
  exists, so notes can be tuned per aspect ratio without hardcoding geometry in
  several places.

## Phase 3: Font and Typography

- Add runtime PDF font selection: `--pdf-font PATH`, `--pdf-mono-font PATH`,
  and `--font-fallback PATH[,PATH...]`.
- Allow theme files to point at PDF font paths while keeping DejaVu as the
  bundled default.
- Preserve `--cjk PATH` as the small-binary path for large CJK fonts.
- Add a font audit command that reports missing glyphs before rendering.
- Improve PDF shaping for scripts that need it only if it can be done without
  making the default binary large.

## Phase 4: Math

- Keep the current Unicode math translator as `--math unicode`.
- Keep `--math svg` as the built-in, no-runtime display-math image mode, and
  add an external-tool mode later for Typst or another user-installed renderer.
- Render display math as images/vector assets consistently across PPTX, ODP,
  PDF, DOCX, and ODT when rich math is enabled.
- Add math diagnostics to `--check` for unsupported constructs.

## Phase 5: More Outputs

Add formats that improve review, publishing, and CI before chasing more office
containers:

- `html`: a self-contained deck for review and sharing.
- `svg`: one SVG per slide, useful for docs and visual tests.
- `png`: one image per slide, useful for screenshots, social posts, and CI.

Do not prioritize native Keynote/Pages until there is a clear user need and a
maintainable writer strategy. PPTX/DOCX import remains the pragmatic path.

## Phase 6: Creative Authoring Features

- Deck Doctor++: design/accessibility warnings for weak slide structure,
  duplicated titles, overpacked tables, missing alt text, and poor reading
  order.
- Auto-layout profiles: `--style dense|balanced|executive|teaching|print`.
- Extend code includes with globbed examples or repository-root aliases if real
  projects need them.
- Continue table work with CSV includes and top-N summaries.
- Accessibility mode: tagged PDF where practical, language metadata, alt text,
  table headers, reading order checks, and PPTX/ODP alt text propagation.
- Theme packs: `md2any theme init`, `md2any theme preview`, shareable brand
  bundles, and sample-slide previews.

## Phase 7: Better Flowing Documents

DOCX and ODT should remain flowing documents, but they do not need to be plain.

- Add document styles that mirror the selected deck theme more closely.
- Add a title page, optional table of contents, section page breaks, headers,
  footers, and callout styles.
- Render slide speaker notes as optional document annotations or appendix
  sections when requested.
- Improve table styling and code-block styling for print/readability.
- Continue refining document profiles with generated field-based TOCs,
  richer callout syntax, and document-specific accessibility metadata.

## Current First Slice

- Ship smart pagination controls and code line-number continuity.
- Fix PDF speaker-notes page sizing so the default notes pages use the deck
  page size instead of hardcoded A4.
- Document the A4 notes mode as an explicit print-oriented option.
- Ship the first render-plan diagnostics: `--emit-ir`, `--emit-plan`, and
  `--trace-layout`.

## Current Second Slice

- Ship `--notes-layout auto|below|side-by-side` so presenter notes can be
  tuned per aspect ratio or forced for a particular speaker workflow.
- Ship `--speaker-package DIR` to emit the editable deck, notes PDF, handout
  PDF, and deterministic JSON manifest in one pass.

## Current Third Slice

- Ship runtime PDF font controls: `--pdf-font`, `--pdf-mono-font`, and
  `--font-fallback`, while keeping bundled DejaVu as the default.
- Let theme files point at PDF font paths through `pdf_font`,
  `pdf_mono_font`, and `font_fallback`.
- Ship `--font-audit` to report fallback glyph use and missing PDF glyphs
  before rendering.

## Current Fourth Slice

- Ship explicit math preprocessing control: `--math unicode` (default) and
  `--math source` for preserving LaTeX source.
- Add math diagnostics to `--check` for unsupported Unicode-translator
  constructs such as matrices, aligned equation line breaks, and unknown
  macros.

## Current Fifth Slice

- Ship `html` as the first native review/publishing output format.
- Add `.html`/`.htm` extension detection, `--format html`, and `--help-html`.
- Render a standalone browser deck from the same paginated IR as PPTX/ODP/PDF,
  including theme colors, layouts, syntax highlighting, image embedding, speaker
  notes toggling, keyboard navigation, and continuation-aware code line numbers.
- Ship `svg` and `png` image-sequence outputs for docs, screenshots, and CI
  artifacts.
- Add `.svg`/`.png` extension detection, `--format svg|png`,
  `--help-svg`, and `--help-png`.
- Use an output directory for image-sequence formats, with deterministic
  `slide-001.svg` / `slide-001.png` file names.
- Ship `--serve-format pdf|html|svg|png` so the live preview server can preview
  the same review/publishing outputs without changing the default PDF loop.

## Current Sixth Slice

- Ship `--doc-style plain|report|handout|speaker-notes` for DOCX/ODT, with
  `report` as the default richer document profile and `plain` as the minimal
  compatibility profile.
- Add themed DOCX/ODT title pages, static contents sections, native page
  headers/footers, image captions, improved table/code/quote styling, and
  optional speaker-notes appendices.
- Extend `--check` into Deck Doctor++ warnings for duplicated titles, missing
  image alt text, dense slides, weak theme contrast, oversized or headerless
  tables, empty columns, and uneven speaker-note coverage.

## Current Seventh Slice

- Ship fenced code includes with `file=path#Lstart-Lend`, resolved relative to
  the Markdown input and preserving source line numbers through pagination and
  code gutters.
- Ship `--table-fit auto|split|transpose|off` and front-matter `table_fit` for
  wide tables.
- In `auto`, transpose compact portrait tables and split very wide tables into
  column groups with the first column repeated as the key column.
- Add table-fit mode to render-plan diagnostics and trace output.

## Current Eighth Slice

- Ship `--math svg` and front-matter `math: svg` as a deterministic built-in
  display-math image mode.
- Convert `$$...$$` display spans into generated SVG data images so
  every existing renderer can use the normal image pipeline.
- Keep inline `$...$` untouched in SVG mode for now; the later rich-math pass
  can add a true inline renderer or an external Typst/TeX bridge.
- Add data-URI image loading so generated assets and future in-memory renderers
  can flow through the same asset path as local and remote images.

## Current Ninth Slice

- Expand the built-in math vocabulary with accents (`\vec`, `\hat`, `\bar`,
  `\tilde`, `\dot`, `\ddot`, `\overline`), grouped text/operator forms
  (`\text`, `\mathrm`, `\operatorname*`), and uppercase `\mathbb`/`\mathcal`
  alphabets.
- Add more common operators, relations, arrows, delimiters, and named
  functions such as `\argmax`, `\argmin`, `\rank`, and `\trace`.
- Let `--math svg` collect multi-line display math blocks and render simple
  matrix, cases, and aligned environments as readable generated SVG images.
- Keep the mode dependency-free; external Typst/TeX-backed rendering remains a
  later opt-in path for publication-grade math layout.

## Current Tenth Slice

- Keep TeX and Typst out of the core path: no runtime dependency, no shell-out,
  and no bundled renderer.
- Parse display math into a small native AST for `--math svg`.
- Render display formulas with deterministic SVG boxes, including stacked
  `\frac{...}{...}` bars, radical bars for `\sqrt{...}`, and improved
  superscript/subscript placement.
- Lay out simple `matrix`, `pmatrix`, `bmatrix`, `cases`, `aligned`, and
  related display environments with aligned columns.
- Add front-matter/API `math_macros` as exact substitutions before the Unicode
  or SVG math pass.
