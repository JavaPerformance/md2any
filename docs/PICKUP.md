# Pickup — md2any Studio (session handoff)

**Saved:** 2026-07-25  
**Branch:** `feat/0.4.0` (tracking `origin/feat/0.4.0`)  
**Status:** Studio stack + product next features landed; **commit stack may exist on branch** — check `git log` / status.  
**Intent:** Ship 0.4.0 when ready (version still `0.3.0` in Cargo.toml until release cut).

Full feature map: [docs/studio.md](./studio.md).

---

## 1. What this workspace contains

### Engine / WASM
| Piece | Path | Notes |
|-------|------|--------|
| In-memory convert API | `src/convert.rs` | `preview_window`, `convert`, `outline`, `lint`, **`slide_images`**; IR deck+frag cache; outline/preview **notes** |
| Brand extract (bytes) | `src/brandkit.rs` | `extract_overlay_bytes` for WASM |
| WASM crate | `crates/md2any-wasm/` | `previewWindow`, `convert`, `lint`, `extractBrand`, **`slideImages`** |
| Studio static host | `src/serve.rs` + `main.rs --studio` | Serves `web/dist`; seed; chat proxy; git status/commit |
| Build | `scripts/build-web.sh` | → `web/dist/` (~7.0M WASM / ~3.0M gzip) |

### Browser UI (`web/`)
| File | Role |
|------|------|
| `index.html` | Chrome: toolbar (Ghost/Notes/Fonts/AI/Git), preview+ghost modes, filmstrip |
| `app.js` | Worker engine, virtualised morph preview, FM sync, adaptive window, **OPFS last-good** |
| `studio-extras.js` | Palette, templates, FSA, brand, filmstrip, rail, share, talk, lint/IR |
| `studio-pro.js` | Ghost HTML+**SVG**, notes panel, **OPFS fonts**, BYO AI, git |
| `worker.js` | WASM off UI thread + preview memo + slideImages |
| `sw.js` + `manifest.webmanifest` + `icons/` | PWA offline shell |
| `style.css` | Dark/light UI + extras/pro styles |

### Docs
- [docs/studio.md](./studio.md) — vision, architecture, feature status table  
- [web/README.md](../web/README.md) — build/deploy  
- [docs/branding.md](./branding.md) — potx extract + studio Brand button  
- [README.md](../README.md) — `--studio` mention  

---

## 2. How to resume (cold start)

```bash
cd /root/md2any
git status -sb
git branch --show-current   # expect feat/0.4.0

# Rebuild studio dist
./scripts/build-web.sh

# Preview
python3 -m http.server -d web/dist 8787
# → http://127.0.0.1:8787/

# Or CLI host (seed + git + AI proxy)
cargo run --features cli -- --studio
cargo run --features cli -- examples/standard-model-lagrangian-a4.md --studio
```

### Quick smoke
```bash
cargo test -p md2any --lib convert
cargo test -p md2any --lib brandkit
curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:8787/studio-pro.js
```

---

## 3. Feature status (shipped vs next)

### Shipped
- Live editor + worker + windowed HTML morph preview  
- Theme/layout/aspect ↔ front-matter  
- Command palette ⌘K, templates, export recipes  
- Rail reorder + rename; filmstrip scrub (+ idle SVG thumbs)  
- FSA open/save/folder + assets map  
- Share URL fragment; review chips; talk mode  
- Brand from `.potx`/`.pptx`  
- Export ghost: **HTML** full export **or SVG** layout window (CLI geometry)  
- Speaker **Notes** panel: draft from IR, AI draft, apply `<!-- notes: -->`  
- **Session Fonts** (OPFS) + `@font-face` in preview; set `title_font`/`body_font`  
- OPFS **last-good** preview recovery on error  
- BYO AI surgical ops; git helper via `--studio`  
- PWA; lint/IR drawers; math badge/macros  
- IR deck+fragment cache; adaptive typing radius; worker memo  
- `md2any --studio` (+ optional seed file)

### Still next
- Audience `variant:` branches  
- Click glyph → font audit  
- Worker dirty-region *parse* (cache full deck, not partial parse)  
- Dual viewport polish / export stamp edge cases  
- True multi-page PNG ghost for entire huge decks (SVG window is the current fidelity path)

---

## 4. Architecture cheat sheet

```text
Browser UI (app.js + studio-extras.js + studio-pro.js)
    │ postMessage
    ▼
Web Worker (worker.js)  ── memoized previewWindow keys
    │
    ▼
md2any-wasm → md2any::convert
    parse → paginate → (cached Deck) → HTML window / slide_images SVG / Office writers
    brandkit::extract_overlay_bytes
```

**Privacy model:** conversion in-tab; share = URL fragment; AI key localStorage; fonts in OPFS; CLI `--studio` chat proxy sends key only to user-chosen endpoint.

---

## 5. Known gotchas

1. **wasm-bindgen version** must match the crate (env used **0.2.126**).  
2. **`file://` will not work** — need HTTP.  
3. **FSA folder/save** needs Chromium; Firefox falls back to download.  
4. **Ghost HTML** runs full convert on idle; **SVG** windows around active slide.  
5. **Deck cache** invalidated on full `convert()` export path.  
6. Package version may still say `0.3.0` while branch is feat/0.4.0 — align on release.  
7. Workspace excludes `/web`, `/crates` from crates.io package — intentional.

---

## 6. One-liner for a new agent

> Resume md2any on branch `feat/0.4.0`. Studio lives in `web/`, `crates/md2any-wasm/`, `src/convert.rs`, `--studio` in serve/main. New: `slideImages`, notes panel, OPFS fonts, SVG ghost, OPFS last-good. Rebuild with `./scripts/build-web.sh`. Spec: `docs/studio.md`. Do not force-push without ask.

---

*End of pickup.*
