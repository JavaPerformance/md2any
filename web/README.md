# md2any Studio (browser / WASM)

Static web app: live Markdown editor + the real md2any engine compiled to
WebAssembly. Everything runs in the visitor’s browser — no upload, no API.

Full feature map and roadmap: [docs/studio.md](../docs/studio.md).

## Build

From the repo root:

```bash
./scripts/build-web.sh
```

Output lands in `web/dist/` (HTML/CSS/JS + PWA + `pkg/*.wasm`).

Requires a recent Rust toolchain and:

```bash
cargo install wasm-bindgen-cli --version 0.2.100
# (pin to the version used by md2any-wasm / Cargo.lock)
```

## Local preview

```bash
./scripts/build-web.sh
python3 -m http.server -d web/dist 8787
# open http://127.0.0.1:8787/
```

Or from the CLI (serves `web/dist` and optionally seeds a document):

```bash
cargo run --features cli -- --studio
cargo run --features cli -- path/to/deck.md --studio
```

(`file://` will not work — ES modules + WASM need HTTP.)

## Deploy to Cloudflare Pages

1. Build: `./scripts/build-web.sh`
2. In Cloudflare Pages, set:
   - **Build command:** `./scripts/build-web.sh`
   - **Build output directory:** `web/dist`
   - **Root directory:** repository root  
   Or upload `web/dist` as a direct deploy.
3. Ensure the build image has a recent Rust toolchain and matching
   `wasm-bindgen-cli`.

Optional `web/_headers` is copied into `dist` for correct `.wasm` MIME type
and caching. PWA files (`manifest.webmanifest`, `sw.js`, icons) ship with the
dist so the app is installable / offline after first load.

## Architecture

| Layer | Role |
|-------|------|
| `web/index.html` + `app.js` | Editor UI, virtualised preview host, localStorage |
| `web/studio-extras.js` | Palette, templates, assets, share, rail reorder, lint/IR panels, talk mode |
| `web/worker.js` | Web Worker — all WASM off the UI thread |
| `web/sw.js` + manifest | PWA offline shell |
| `crates/md2any-wasm` | `previewWindow`, `convert`, `lint`, theme/layout lists |
| `md2any::convert::preview_window` | Full parse + outline; **HTML only for a slide window** |
| Virtual assets | Images as path → base64 (no filesystem in WASM) |

### Preview scaling model

```text
keystroke (debounced ~60ms)
  → worker: parse whole deck (breaks / pagination)
  → worker: emit HTML only for active ± 3 slides
  → UI: patch only slides whose contentKey changed
  → spacers stand in for off-window slides
Download export
  → worker: full pptx/pdf/… pipeline (OK to wait)
```

A 300-slide deck does **not** mount or morph 300 DOM nodes on each edit.
Parse/paginate remains O(deck); only HTML generation and DOM are windowed.

### Studio extras (shipped)

| Feature | How |
|---------|-----|
| Command palette | `⌘/Ctrl+K` |
| Templates | Empty-state gallery + palette |
| Export recipes | Board pack (PDF+PPTX), all formats |
| Slide rail reorder | Drag items; rewrites `---` sections |
| Filmstrip scrub | Bottom strip from IR titles / kinds |
| Open / Save | File System Access (`⌘S`); download fallback |
| Open folder | Loads `.md` + `assets/` into the session |
| Brand from `.potx` | WASM extract → apply `style:` or download YAML |
| Export ghost | Idle HTML export side-by-side with live preview |
| BYO AI | Local key → surgical slide ops (proxy via `--studio`) |
| Git helper | Commit message + command; host commit when seeded |
| IR / adaptive cache | Deck+fragment cache; tighter window while typing |
| Share snapshot | URL hash `#md2any.g1…` (fragment, local only) |
| Assets drop/paste | Images → `assets/…` virtual map |
| Math badge | Cycle `unicode` / `svg` / `source` |
| Macro palette | From front-matter `math_macros` |
| IR inspector + lint | Drawer panels via WASM |
| Talk mode | `T` — fullscreen live preview |
| Review chips | `<!-- @review: … -->` comments |
| Diff since export | Status bar stamp |

## Keyboard map

| Shortcut | Action |
|----------|--------|
| `⌘/Ctrl+K` | Command palette |
| `⌘/Ctrl+S` | Save markdown |
| `⌘/Ctrl+Shift+S` | Export PDF |
| `⌘/Ctrl+Shift+E` | Export PPTX |
| `T` | Talk mode (when not typing) |
| `Escape` | Close palette / panels / talk mode |

## Privacy model

- Conversion runs in **your browser** (WASM worker).
- Default persistence is **localStorage** + download.
- Share links encode the document in the **URL fragment** (not sent to a server by this app).
- No analytics required for core function.

## Limits

- No remote `http(s)` image fetch in-browser (use uploads / paste / `data:` URIs)
- No Mermaid/dot shell-out (code blocks stay as code)
- No AI dock (CLI `--serve --edit` still has that)
- Pathological megadoc markdown may still need future incremental parse
