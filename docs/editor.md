# The live editor

```sh
md2any deck.md --serve --edit          # editor + live preview at http://localhost:8421
md2any deck.md --serve --edit --port 8800
md2any deck.md --serve --edit --serve-format pdf   # preview as PDF instead of HTML
```

`--serve --edit` opens a single-binary, zero-dependency editor in your browser:
markdown on the left, a live preview on the right, and an AI assistant docked
along the bottom. Everything is served from the md2any binary — no Node, no
build step, no external runtime. Edits autosave to the source file, so your
normal `--watch`/git workflow keeps working.

## Layout

| Region | What it does |
|---|---|
| **Top bar** | Wordmark, a **status pill** (saved / editing / saving), the current **slide position** (`12 / 30`), the preview **format**, a **Generate ▾** export menu, and a **🎨 Style** button. |
| **Editor** (left) | A plain-markdown textarea. Type normally; there's no special syntax beyond md2any's own (see the [README](../README.md)). |
| **Preview** (right) | The rendered deck. The slide under your caret gets a soft accent **glow ring** and scrolls into view as you type. |
| **AI dock** (bottom) | Chat with the deck, run quick actions, target a slide, and find images. Click the bar to expand/collapse. |
| **Style panel** | Slides in from the right (🎨). Theme, colours, and sizes — written straight into front-matter. |

## Live preview

The preview is **true live-DOM editing**: on each rebuild md2any *morphs* the new
render into the existing preview DOM, so only the changed slide's nodes update.
Scroll position and already-loaded images on every other slide are preserved —
no flash, no jump to the top.

- **Caret ↔ slide sync** is two-way: move the caret and the matching slide
  highlights; click a slide in the preview and the caret jumps to its source.
- **Autosave**: edits are debounced and written back to the source file; the
  file watcher rebuilds and the preview refreshes.
- **Preview format** defaults to HTML; `--serve-format pdf|svg|png` previews the
  real export pipeline instead (heavier, but pixel-accurate to that format).

## Style panel (🎨)

Theme, accent/background/title colours, title and body sizes, and aspect ratio.
Every control writes a value into the deck's front-matter `style:` block (or
`theme:` / `aspect:`), so the panel is just a friendly front end to text you
could type yourself — and it round-trips: edit the front-matter by hand and the
panel reflects it.

## AI assistant dock

The dock sends the model your **full document**, a numbered **slide list**, and
the **slide you've selected** (click any slide to target it — "this slide" then
means that one). It replies with **surgical, slide-addressed edits** you apply
with one click — it does *not* rewrite the whole deck for a small change.

- **Quick-action chips**: proofread, make concise, add a summary slide, add
  speaker notes, suggest a title, and **🖼 Find an image…**.
- **Apply / content-loss guard**: edits show an **✓ Apply** button. If an edit
  would drop existing prose/images you didn't ask to remove, the editor asks for
  confirmation first (models occasionally regenerate a slide and forget content).
- **Configuration**: the dock speaks an OpenAI-compatible chat API. Set the
  endpoint/model with `--ai-endpoint` / `--ai-model`, and the key via
  `$MD2ANY_API_KEY` or a gitignored key file (e.g. `grok-api.key`). Drop the
  default `ai` feature for a network-free binary.

### Finding and inserting real photos

The model has a `search_images` tool, and there's a manual **🖼 Find an image…**
panel too. Both query the same backend:

| Source | Needs a key? |
|---|---|
| Wikimedia Commons | no |
| Openverse | no |
| Unsplash | `unsplash-api.key` |
| Pexels | `pexels-api.key` |

Results carry the image URL, **licence, and author**. When you pick one (or the
AI inserts one), md2any downloads it into `assets/` next to the deck, rewrites
the link to the local path, and the AI adds a short credit line — so the deck
stays **self-contained** and correctly attributed. WebP originals are kept
(md2any decodes WebP natively).

> Tip: you can also localize a whole deck from the command line with
> `md2any deck.md --localize`, which downloads every remote `http(s)` image into
> `assets/` and rewrites the links.

## HTTP endpoints

The server is a small HTTP API you can script against:

| Method · path | Purpose |
|---|---|
| `GET /` | the editor page |
| `GET /version` | current build version (the preview polls this for hot reload) |
| `GET /source` · `POST /source` | read / write the deck markdown |
| `GET /export?format=pptx\|odp\|pdf\|docx\|odt\|html\|svg\|png` | download the deck in any format |
| `POST /chat` | one streamed AI turn (`{messages, doc, slides, active}` → NDJSON deltas) |
| `GET /image-search?q=…` | image search results as JSON |
| `POST /localize` | download remote images in the posted doc into `assets/`, return the rewritten doc |
| `GET /slides/NNN.svg\|png` | individual slide images (in image preview mode) |

## Notes

- **Offline**: everything but remote image fetching and the AI dock works
  without a network. Keyless image sources and the AI need internet.
- **One file**: the editor edits the first input file; multi-file concat decks
  are preview-only.
- **Security**: the server binds to localhost and serves your local deck — it is
  a personal editing tool, not a public server.
