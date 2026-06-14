# md2any torture-test findings

Broad stress test of the renderer (2026-06-14). Six domains — tables/lists/columns,
code/syntax, typography/unicode, images/SVG, themes/layouts/aspects, and multi-format
robustness — each rendered to PDF/PNG (and cross-checked in HTML/Office formats) and
visually inspected.

**Multi-format output is solid:** 72 artifacts across all 8 formats (pptx, odp, pdf,
docx, odt, html, svg, png) — no crashes, no corrupt archives, no dropped content.
All defects below are in the slide layout/render layer.

Status legend: ☐ open · ☑ fixed · ◐ in progress · ⊘ won't-fix / larger effort

---

## HIGH

- ☑ **H1 — Long titles overflow & overprint the footer.** Fixed across **all**
  title layouts (Clean/Studio/Frame/Bold): `fit_hero_size` shrinks the hero title
  to fit above the subtitle/footer; Bold no longer drops the title off the top of
  its accent block (`src/pdf.rs`). Subtitle/author stay visible.

- ☑ **H2 — Wrapped content headings collide with body.** Fixed: `render_content_slide`
  measures the wrapped title line count and pushes the underline + body down by the
  extra lines (`src/pdf.rs`). Was: fixed-height title band.

- ☑ **H3 — Section-divider long titles clipped off the bottom.** Fixed: section
  divider (Clean/Frame) now auto-shrinks via `fit_hero_size` (`src/pdf.rs`).

- ☑ **H4 — Inline `<svg>…</svg>` silently dropped.** Fixed: the parser accumulates
  inline SVG across HTML events and routes it through the image pipeline as a
  base64 `data:image/svg+xml` block, rasterised like external SVG (`src/parser.rs`).

- ☑ **H5 — Table column alignment ignored** (`:---:`, `---:`). Fixed: added
  `ir::ColumnAlign` + `Table.aligns`, captured from the GFM delimiter row; honoured
  in **PDF / SVG / PNG / HTML**. (Office formats — pptx/odp/odt/docx — thread the
  data but don't yet apply native cell alignment; see L7.)

- ☑ **H6 — Tabs in code → tofu boxes + broken indentation.** Fixed: code lines
  expand tabs to 4-column tab stops at parse time, fixing every backend
  (`src/parser.rs`).

## Known-limitation class (larger efforts)

- ⊘ **H7 — RTL (Arabic/Hebrew) not reordered.** No bidi; Hebrew reversed, Arabic
  left-aligned. Needs a bidi pass — larger effort.

- ⊘ **H8 — CJK + emoji → tofu in PDF.** Only DejaVu embedded; no CJK/emoji fallback
  font. HTML preserves them. Needs a fallback font pipeline — larger effort.

## MEDIUM

- ☑ **M1 — Nested blockquotes collapse.** Fixed: the quote accumulator only
  resets on the outermost `>`, so nested levels keep the outer paragraphs
  (`src/parser.rs`).

- ☑ **M2 — Long unbreakable tokens overflow the right edge.** Fixed: PDF/SVG
  wrappers hard-break a token wider than the line into character chunks; HTML
  uses `overflow-wrap: anywhere` (`src/pdf.rs`, `src/svg.rs`, `src/html.rs`).

- ☑ **M3 — `{width=N%}` > 100 leaks literal `{width=…}` as body text** (+ spurious
  slide). Fixed: `parse_width_pct` now clamps to 1..=100 instead of returning `None`
  (`src/parser.rs`).

- ☑ **M4 — `:::` columns: a leading/extra marker dumps everything into the right
  column** (empty left). Fixed: `coalesce_columns` now splits on all dividers and
  drops empty segments, so the fence form `::: … ::: … :::` works (`src/paginate.rs`).

- ☑ **M5 — Task-list items render a bullet AND a checkbox** (`● ☐ task`). Fixed:
  added `ListItem::is_task()`; bullet suppressed in pdf/svg/html/pptx render_list.
  ODT/DOCX use native list styles and still show both — needs a no-bullet list
  style declaration (follow-up, see L7).

- ☐ **M6 — Pagination over-splits in 16:9** — image+caption and tables split with
  slides ~60% empty (a4 fits them). Same family as the math-image weight fix already
  landed; the height budget is too conservative for several block types in 16:9.

## LOW / polish

- ☐ L1 — Code: leading/trailing blank lines inside a fence kept verbatim.
- ☐ L2 — Rust `'static` lifetime mis-highlighted as a char/string literal.
- ☐ L3 — `&nbsp;` not decoded (renders literal `nbsp`).
- ☐ L4 — `₿` (U+20BF) tofu; other currency symbols fine.
- ☐ L5 — Misleading "aspect too small" error for ratios like `16:10`/`1:1` (only
  `16:9`/`4:3`/`9:16` presets accepted).
- ☐ L6 — TOC lists only H1 sections.

---

## Fix order

Cheap/safe first, then the big UX wins:
M3 (width clamp) → M5 (task-list bullet) → M4 (`:::` leading marker) →
H1/H2/H3 (title & heading auto-fit + body push-down) → H4 (inline SVG) →
H5 (table alignment, needs IR field) → M6 (16:9 pagination budget) →
M1 (nested blockquotes) → M2 (token overflow) → low/polish.
H7/H8 (RTL, CJK/emoji) are larger and tracked separately.
