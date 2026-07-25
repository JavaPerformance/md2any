# md2any Studio

Browser UI for the same conversion engine as the CLI: **one markdown source →
real PPTX / PDF / DOCX / ODP / ODT / HTML**, running fully offline via
WebAssembly. No upload. No Office install. No Chromium.

```bash
./scripts/build-web.sh
python3 -m http.server -d web/dist 8787
# open http://127.0.0.1:8787/

# or from the CLI (same dist, optional seed file):
cargo run --features cli -- --studio
cargo run --features cli -- examples/standard-model-lagrangian-a4.md --studio
```

Cloudflare Pages: build command `./scripts/build-web.sh`, output `web/dist`.

Environment: `MD2ANY_STUDIO_DIR` or `--studio-dir` overrides the dist path.
Default port is **8787** (or `--port` if not the serve default).

---

## Positioning

| | md2any Studio | Typical “md → PDF” web tools |
|--|---------------|------------------------------|
| PPTX | **Editable native** | Often image-baked or missing |
| PDF | Native writer | Print dialog / headless Chrome |
| Privacy | **In-tab WASM** | Upload to a server |
| Source of truth | **Same IR as CLI** | Parallel, lossy pipeline |

**Taglines we own:**

- Editable PowerPoint — not a screenshot of your markdown.
- Same IR as the CLI. No second truth.
- Your deck never left the device.

---

## Architecture (shipped)

```text
┌─────────────────────────────────────────────────────────────┐
│  Studio UI (static: app.js + studio-extras.js)              │
│  editor · rail · virtualised preview · palette · panels     │
└──────────────────────────┬──────────────────────────────────┘
                           │ postMessage
                           ▼
┌─────────────────────────────────────────────────────────────┐
│  Web Worker (worker.js)                                     │
│  previewWindow · convert · lint                             │
└──────────────────────────┬──────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────────┐
│  md2any-wasm → md2any::convert                              │
│  parse → paginate → HTML window / Office writers            │
└─────────────────────────────────────────────────────────────┘
```

### Preview scaling

| Layer | Cost model |
|-------|------------|
| Parse / paginate | O(deck) in the worker (UI stays responsive) |
| HTML generation | **O(window)** — active ± radius, pinned at ends |
| DOM | In-place **morph** per changed slide (`contentKey`) |
| Export | Full deck in worker (OK to wait) |

---

## Feature map

Status: **done** = in the current studio build · **partial** = scaffold / first cut · **next** = designed, not fully built

### Thinking surface

| Idea | Status | Notes |
|------|--------|--------|
| Live editor + virtualised preview | **done** | Worker + windowed HTML |
| Caret ↔ slide halo + click-to-source flash | **done** | |
| Theme / layout / aspect ↔ front-matter | **done** | Two-way |
| Slide rail reorder (drag) | **done** | Rewrites `---` sections in markdown |
| Rename slide from rail | **partial** | Double-click title → edits first heading line |
| Talk mode (fullscreen preview) | **done** | `T` or palette |
| Filmstrip / timeline scrub | **done** | Bottom strip; SVG thumbs when idle job fills |

### Fidelity & trust

| Idea | Status | Notes |
|------|--------|--------|
| Format truth panel | **done** | Palette → “What each export keeps” |
| Export recipes (board pack, etc.) | **done** | Multi-download sequences |
| Dual viewport / export ghost | **done** | HTML export + SVG layout modes (PPTX-like geometry) |
| Diff since last export | **done** | Status bar: not exported / in sync / changed |

### Assets & brand

| Idea | Status | Notes |
|------|--------|--------|
| Drop / paste images → virtual assets | **done** | `assets/…` paths + in-memory map |
| Clipboard tables → markdown | **partial** | TSV/CSV paste → pipe table |
| Brand kit from `.potx` | **done** | WASM `extractBrand` → `style:` + download YAML |
| Session font drawer | **done** | OPFS + `@font-face` preview; `title_font` / `body_font` |

### AI (local key)

| Idea | Status | Notes |
|------|--------|--------|
| Surgical slide edits | **done** | BYO key → browser or `md2any --studio` proxy |
| Speaker notes from IR | **done** | Notes panel; draft from IR; optional AI draft |
| Audience variants | **next** | Front-matter `variant:` branches |

### Collaboration (still private-first)

| Idea | Status | Notes |
|------|--------|--------|
| Share snapshot in URL hash | **done** | `#md2any1.<base64url>` local only |
| `<!-- @review: … -->` chips | **done** | Panel lists review comments |
| File System Access open / save | **done** | `showOpenFilePicker` / `showSaveFilePicker` |
| Open folder + `assets/` | **done** | Directory picker; loads images into virtual map |
| Git commit helper | **done** | Copy command; live status/commit via `--studio` seed |

### Math

| Idea | Status | Notes |
|------|--------|--------|
| Front-matter `math: svg` honored | **done** | Same as CLI |
| Full-page ` ```math ` → SVG layout | **done** | + system STIX when available |
| Math mode badge + cycle | **done** | |
| Macro palette from front-matter | **done** | Insert at caret |
| Click glyph → font audit | **next** | |

### Product shell

| Idea | Status | Notes |
|------|--------|--------|
| Command palette `⌘K` / `Ctrl+K` | **done** | |
| Template gallery | **done** | Empty state + palette |
| Export menu + recipes | **done** | |
| PWA install (manifest + SW) | **done** | Offline after first load |
| IR inspector | **done** | Outline JSON drawer |
| Lint / “stress deck” | **done** | `lint` via WASM |
| Print stylesheet | **done** | Preview-quality handout |
| `md2any --studio` handshake | **done** | Serves `web/dist`; optional INPUT as seed |

### Performance

| Idea | Status | Notes |
|------|--------|--------|
| Worker IR cache (deck + fragment) | **done** | Reuse parse/paginate; HTML frag cache per slide |
| Worker request memo | **done** | Identical previewWindow keys |
| Adaptive fidelity while typing | **done** | Radius 1 while typing → 3 when idle |
| OPFS last-good window | **done** | Restores outline+HTML window if preview fails |

---

## Keyboard map (studio)

| Shortcut | Action |
|----------|--------|
| `⌘/Ctrl+K` | Command palette |
| `⌘/Ctrl+S` | Save markdown (FSA or download) |
| `⌘/Ctrl+Shift+S` | Export PDF |
| `⌘/Ctrl+Shift+E` | Export PPTX |
| `T` | Talk mode (when not typing in editor) |
| `Escape` | Close palette / panels / talk mode |
| `?` | Open palette on “help” |

---

## Privacy model

- Conversion runs in **your browser** (WASM worker).
- Default save target is **localStorage** + optional download.
- Share links encode the document in the **URL fragment** (not sent to a server by this app). Treat them like secrets if the content is sensitive.
- No analytics required for core function.

---

## Related docs

- [docs/editor.md](./editor.md) — CLI `--serve --edit`
- [docs/theming.md](./theming.md) — themes / overlays
- [docs/branding.md](./branding.md) — brand kits from PPTX
- [web/README.md](../web/README.md) — build & deploy
