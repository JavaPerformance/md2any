# Branding & corporate templates

Most organisations have a **deck template** — a PowerPoint `.potx`, a brand book,
a set of colours and fonts that every presentation must follow. This guide
explains how md2any handles that.

## The model: generate, don't host

A PowerPoint template (`.potx`) is a *container*: you open it and pour content
into its slide masters and placeholders. md2any works the other way round — it
is a **generator**. It builds every artifact (PPTX, ODP, PDF, DOCX, ODT, HTML,
SVG, PNG) from scratch on each run, so it never opens or depends on someone
else's master slides.

That is deliberate. A `.potx` would only brand **one** of the eight outputs.
md2any's whole job is *one Markdown source → many on-brand artifacts,
deterministically, in CI*. So branding is **declarative**: you describe the
brand once, in a small YAML file, and every format honours it.

```bash
md2any deck.md --theme light --theme-file brand.yaml --logo logo.png -o deck.pptx
md2any deck.md --theme light --theme-file brand.yaml --logo logo.png -o deck.pdf
# …same brand.yaml for odp / docx / odt / html / svg / png
```

## Already have a `.potx`? Extract a starter overlay

The part of a template that actually carries the brand — its **colour scheme**
and **font scheme** — lives in one well-specified XML part inside the file
(`ppt/theme/theme1.xml`). md2any can read it and emit a starter `brand.yaml`:

```bash
md2any theme extract corporate.potx -o brand.yaml     # also accepts .pptx
md2any theme extract corporate.potx                   # print to stdout
```

In the **browser studio**, use **Brand** (or the command palette → “Import brand”):
pick a `.potx`/`.pptx` — the same extractor runs in WASM, shows swatches, and can
**Apply to document** (writes a front-matter `style:` block) or download `brand.yaml`.
Nothing is uploaded.

The output is a commented overlay you can use immediately and tweak later:

```yaml
# md2any brand overlay — extracted from corporate.potx

bg: "#FFFFFF"          # lt1 — slide background
body_color: "#000000"  # dk1 — body text
title_color: "#44546A" # dk2 — slide titles
accent: "#4472C4"      # accent1 — primary brand colour
accent_soft: "#ED7D31" # accent2 — secondary / card fill
divider: "#E7E6E6"     # lt2 — rules / dividers
link: "#0563C1"        # hlink — hyperlinks
section_bg: "#4472C4"  # section-divider background
on_accent: "#FFFFFF"   # text drawn on the accent

title_font: "Georgia"  # majorFont
body_font: "Verdana"   # minorFont
```

### What is and isn't carried over

| From the template | Mapped to | Notes |
|---|---|---|
| `clrScheme` → `lt1` | `bg` | slide background |
| `clrScheme` → `dk1` | `body_color` | body text |
| `clrScheme` → `dk2` | `title_color` | slide titles |
| `clrScheme` → `accent1` | `accent`, `section_bg` | primary brand colour |
| `clrScheme` → `accent2` | `accent_soft` | secondary / card fill |
| `clrScheme` → `lt2` | `divider` | rules and dividers |
| `clrScheme` → `hlink` | `link` | hyperlinks |
| `fontScheme` → major Latin | `title_font` | heading typeface |
| `fontScheme` → minor Latin | `body_font` | body typeface |

**Not** imported (by design — they're either format-specific or not in the
theme part): slide-master geometry and placeholders, decorative shapes,
the logo, and the monospace/code font (templates don't define one — set
`mono_font` yourself). The extractor gives you the brand *palette and fonts*;
you compose the rest with `--layout`, `--logo`, and the per-slide controls.

## The font story (read this if fonts matter)

| Output | How fonts resolve |
|---|---|
| PPTX / ODP / DOCX / ODT | The overlay **names** `title_font` / `body_font` / `mono_font`. They render correctly **if installed** on the machine that opens the file — exactly like a native PowerPoint template. |
| PDF | PDF is self-contained, so a name isn't enough — the glyphs must be **embedded**. Point `pdf_font` / `pdf_mono_font` at `.ttf`/`.otf` files you can ship. Without them PDF falls back to the bundled DejaVu family. |
| HTML | Uses the named fonts if the viewer has them; otherwise the browser's defaults. |

```yaml
# Add to brand.yaml so PDF matches the rest:
pdf_font: "fonts/BrandSans.ttf"
pdf_mono_font: "fonts/BrandMono.ttf"
font_fallback: "fonts/NotoSans.ttf"   # for glyphs the brand font lacks
```

Run `md2any deck.md --pdf-font fonts/BrandSans.ttf --font-audit` to see whether
a font covers every glyph your deck uses.

## Building a brand from scratch (no `.potx`)

You don't need a PowerPoint file. Any of these is a complete `brand.yaml`:

```yaml
accent: "#C8102E"      # the one colour most brands actually enforce
title_font: "Arial"
body_font: "Arial"
```

Only the keys you set are applied; everything else inherits the base `--theme`
(`light`, `dark`, or one of the built-in palettes — run `md2any --list-themes`).
The full set of overlay keys — including syntax-highlighting colours and the
layout geometry knobs — is documented in [docs/theming.md](theming.md).

## Putting it together for a team

1. `md2any theme extract corporate.potx -o brand.yaml` (or hand-write it).
2. Add brand fonts under `fonts/` and wire `pdf_font` / `pdf_mono_font`.
3. Pick the structural look with `--layout clean|studio|frame|bold`.
4. Drop the logo in with `--logo`.
5. Commit `brand.yaml`, `fonts/`, and the logo to the repo, and wrap the call
   in a script or shell alias so every deck is one command:

```bash
# bin/deck — the house style, applied to any markdown file
md2any "$1" --theme light --theme-file brand.yaml \
  --pdf-font fonts/BrandSans.ttf --logo assets/logo.png -o "${1%.md}.pptx"
```

Now "on brand" is a flag, not a chore — and it's identical across PPTX, PDF, and
every other format md2any emits.

See also: [docs/theming.md](theming.md) for the full overlay reference, and
[docs/editor.md](editor.md) — the in-browser editor's 🎨 style panel writes the
same keys for you.
