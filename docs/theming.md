# Theming md2any

Three layers, applied in order, all optional:

1. **A built-in theme** — `--theme NAME` (or `theme: NAME` in front-matter).
2. **A layout** — `--layout clean|studio|frame|bold` (geometry/chrome; orthogonal to themes).
3. **A theme-file overlay** — `--theme-file brand.yaml`, applied on top of the
   chosen theme so you only override what you want.

## Built-in themes

`--list-themes` prints them:

| name | base | feel |
|---|---|---|
| `light` | light | the default — clean, blue accent |
| `dark` | dark | deep navy, cyan accent |
| `corporate` | light | navy + Georgia titles, centred |
| `sepia` | light | warm paper, serif |
| `contrast` | light | high-contrast black / red / yellow |
| `midnight` | dark | indigo, violet accent |
| `terminal` | dark | green-on-black, monospaced |
| `pastel` | light | soft pink / violet |

Each gallery theme is just a palette layered on a light or dark base, so it
inherits all geometry and sizing and combines freely with any `--layout`,
`--aspect`, and theme-file overlay.

## Theme-file reference (`--theme-file FILE.yaml`)

Every key is optional; only the keys present are applied. Colours accept
`#RRGGBB`, `RRGGBB`, `#RGB`, or `RGB`.

```yaml
# Colours
bg:            "#FFFFFF"   # slide background
title_color:   "#0F172A"   # slide/section titles
body_color:    "#334155"   # body text
muted_color:   "#94A3B8"   # footers, captions
accent:        "#0EA5E9"   # rules, bullets, links
accent_soft:   "#E0F2FE"   # table header band, soft fills
divider:       "#E2E8F0"   # rules and borders
code_bg:       "#F1F5F9"
code_text:     "#1E293B"
code_accent:   "#0369A1"
section_bg:    "#0F172A"   # full-bleed section dividers
section_text:  "#F8FAFC"
link:          "#0EA5E9"
on_accent:     "#FFFFFF"   # text drawn on accent fills

# Typography
title_font:    "Georgia"        # HTML / Office face for titles
body_font:     "Calibri"
mono_font:     "Consolas"
pdf_font:      "fonts/Brand.ttf"      # PDF sans/body (TTF/OTF)
pdf_mono_font: "fonts/BrandMono.ttf"  # PDF mono/code
font_fallback: ["fonts/NotoCJK.ttf"]  # PDF per-glyph fallbacks
title_size:    2800   # centipoints (28.0 pt)
body_size:     1800
code_size:     1500
hero_size:     5400   # title-slide / section hero

# Layout knobs
title_align:   center   # left (default) | center — aligns ## slide titles

# Syntax highlighting
syntax:
  keyword:   "#9333EA"
  string:    "#16A34A"
  number:    "#C2410C"
  comment:   "#94A3B8"
  function:  "#2563EB"
  type:      "#0891B2"
  attribute: "#DC2626"
```

Front-matter can also carry a `theme_file:` path, and individual keys via the
`theme:`/`aspect:`/`layout:` front-matter fields.

## Notes

- Code-block colours follow the theme's background luminance automatically
  (light bg → light code theme, dark bg → dark); override with `--code-theme`.
- PDF embeds DejaVu Sans/Mono by default; `pdf_font`/`pdf_mono_font` and
  `font_fallback` replace or extend those faces. HTML/Office formats use the
  `*_font` family names directly.
