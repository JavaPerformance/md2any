/**
 * md2any Studio — static browser host for the WASM conversion engine.
 * Drop web/dist on Cloudflare Pages (or any static host).
 *
 * Preview architecture:
 *  - Engine work runs in a Web Worker (UI thread stays responsive)
 *  - Only a window of slides is HTML-rendered (virtualised)
 *  - DOM patches only slides whose contentKey changed
 *  - Full-deck export still uses the complete pipeline on demand
 */

const STORAGE_KEY = "md2any-studio:v1";
/** Debounce after last keystroke before requesting a preview. */
const PREVIEW_MS = 60;
/** How many slides before/after the active one to HTML-render (idle). */
const WINDOW_RADIUS = 3;
/** Tighter window while typing — adaptive fidelity. */
const WINDOW_RADIUS_TYPING = 1;
/** Fallback slide block height (px) before we measure a real slide. */
const DEFAULT_SLIDE_BLOCK = 440;
/** After last keystroke, restore full window radius. */
const TYPING_IDLE_MS = 450;

// ---------------------------------------------------------------------------
// Worker-backed engine
// ---------------------------------------------------------------------------
class Engine {
  constructor() {
    this.worker = new Worker(new URL("./worker.js", import.meta.url), {
      type: "module",
    });
    this.seq = 0;
    this.pending = new Map();
    this.worker.onmessage = (ev) => {
      const msg = ev.data || {};
      const slot = this.pending.get(msg.id);
      if (!slot) return;
      this.pending.delete(msg.id);
      if (msg.ok) slot.resolve(msg.result);
      else slot.reject(new Error(msg.error || "worker error"));
    };
    this.worker.onerror = (e) => {
      console.error("engine worker error", e);
    };
  }

  call(op, payload = {}) {
    const id = ++this.seq;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.worker.postMessage({ id, op, ...payload });
    });
  }

  init() {
    return this.call("init");
  }

  previewWindow(args) {
    return this.call("previewWindow", args);
  }

  convert(args) {
    return this.call("convert", args);
  }

  lint(args) {
    return this.call("lint", args);
  }

  extractBrand(args) {
    return this.call("extractBrand", args);
  }

  slideImages(args) {
    return this.call("slideImages", args);
  }
}

const engine = new Engine();

import { installStudioExtras } from "./studio-extras.js";
import { installStudioPro } from "./studio-pro.js";

const SAMPLE = `---
title: Welcome to md2any Studio
author: You
theme: midnight
layout: clean
aspect: 16:9
---

# Welcome to md2any Studio

Markdown in the browser → **real** PowerPoint, PDF, and Word.

No upload. No server. The full md2any engine runs as WebAssembly in this tab.

---

## Why this is different

- **Editable PPTX** — not screenshots baked into slides
- **Native PDF** — no print dialog, no Chromium
- **DOCX / ODT / ODP** — same source, many formats
- **Themes & layouts** — pick from the toolbar

---

## Math works too

Euler's identity:

$$e^{i\\pi} + 1 = 0$$

And a Lorentz factor:

$$\\gamma = \\frac{1}{\\sqrt{1 - v^2/c^2}}$$

---

## Code & tables

\`\`\`rust
fn main() {
    println!("hello from md2any");
}
\`\`\`

| Format | Editable | Offline |
|--------|:--------:|:-------:|
| PPTX   | yes      | yes     |
| PDF    | —        | yes     |
| DOCX   | yes      | yes     |

---

## Next steps

1. Edit this markdown on the left
2. Watch the live preview update
3. **Download** PPTX or PDF from the menu
4. Prefer a CLI? \`cargo install md2any\`
`;

/** DOM by id. Accepts `foo` or `#foo` (extras/pro historically mixed both). */
const $ = (id) => {
  if (id == null || id === "") return null;
  const key = String(id);
  return document.getElementById(key.startsWith("#") ? key.slice(1) : key);
};

const state = {
  ready: false,
  theme: "midnight",
  layout: "clean",
  aspect: "16:9",
  assets: {}, // path -> base64
  activeSlide: 1,
  previewTimer: null,
  outlineTimer: null,
  exporting: false,
  /** True once the preview iframe has a real deck document we can morph into. */
  previewReady: false,
  /** Last outline slides (for caret → slide mapping). */
  slides: [],
  /** Known control values (filled at init from the WASM engine). */
  themeList: [],
  layoutList: [],
  aspectList: ["16:9", "4:3", "a4", "letter"],
  /** Suppress FM→control sync while we rewrite front-matter from a dropdown. */
  applyingControls: false,
  flashTimer: null,
  flashScrollTimer: null,
  /** Monotonic generation — drop stale worker responses. */
  previewGen: 0,
  outlineTimer: null,
  /** Until this timestamp, use a smaller HTML window (adaptive fidelity). */
  typingUntil: 0,
  preview: {
    structureKey: null,
    /** @type {Map<number, { key: string, el: Element }>} */
    mounted: new Map(),
    shellReady: false,
    slideHeight: DEFAULT_SLIDE_BLOCK,
    cssKey: null,
    lastFrom: null,
    lastTo: null,
    /** Optional 0-based center from rail click / preview scroll (overrides caret). */
    focusIndex: null,
    scrollTimer: null,
  },
};

// ---------------------------------------------------------------------------
// Front-matter ↔ toolbar sync
// Theme / layout / aspect live in YAML front-matter *and* the dropdowns.
// Dropdown changes rewrite the markdown; markdown edits update the dropdowns.
// ---------------------------------------------------------------------------

/** Parse a simple `key: value` front-matter block (enough for theme/layout/aspect). */
function parseFrontMatter(md) {
  const lines = md.split("\n");
  if (!lines.length || lines[0].trim() !== "---") {
    return { fields: {}, end: -1, lines };
  }
  for (let i = 1; i < lines.length; i++) {
    if (lines[i].trim() === "---") {
      const fields = {};
      for (let j = 1; j < i; j++) {
        const m = lines[j].match(/^([A-Za-z0-9_-]+)\s*:\s*(.*?)\s*$/);
        if (!m) continue;
        let v = m[2];
        // Strip matching quotes; ignore inline comments only when value is bare.
        if (
          (v.startsWith('"') && v.endsWith('"')) ||
          (v.startsWith("'") && v.endsWith("'"))
        ) {
          v = v.slice(1, -1);
        } else {
          const hash = v.indexOf(" #");
          if (hash >= 0) v = v.slice(0, hash).trim();
        }
        fields[m[1]] = v;
      }
      return { fields, end: i, lines };
    }
  }
  return { fields: {}, end: -1, lines };
}

/**
 * Set or insert a top-level front-matter field. Creates a FM block if missing.
 * Returns the new markdown string.
 */
function setFrontMatterField(md, key, value) {
  const lines = md.split("\n");
  if (!lines.length || lines[0].trim() !== "---") {
    const body = md.startsWith("\n") ? md : `\n${md}`;
    return `---\n${key}: ${value}\n---${body}`;
  }
  let end = -1;
  for (let i = 1; i < lines.length; i++) {
    if (lines[i].trim() === "---") {
      end = i;
      break;
    }
  }
  if (end < 0) {
    return `---\n${key}: ${value}\n---\n${md}`;
  }
  let found = false;
  for (let i = 1; i < end; i++) {
    const m = lines[i].match(/^([A-Za-z0-9_-]+)\s*:/);
    if (m && m[1] === key) {
      lines[i] = `${key}: ${value}`;
      found = true;
      break;
    }
  }
  if (!found) {
    lines.splice(end, 0, `${key}: ${value}`);
  }
  return lines.join("\n");
}

/** Read theme/layout/aspect from markdown into state + dropdowns (if recognized). */
function syncControlsFromMarkdown() {
  if (state.applyingControls) return;
  const { fields } = parseFrontMatter($("editor").value);
  const apply = (key, list, selId) => {
    const raw = fields[key];
    if (raw == null || raw === "") return;
    // Case-insensitive match against known options.
    const hit =
      list.find((x) => x.toLowerCase() === String(raw).toLowerCase()) ||
      (list.length === 0 ? raw : null);
    if (!hit) return;
    if (state[key] !== hit) state[key] = hit;
    const sel = $(selId);
    if (sel && sel.value !== hit) {
      // Setting .value does not fire `change`.
      if ([...sel.options].some((o) => o.value === hit)) {
        sel.value = hit;
      } else {
        // Document uses a value not yet in the list — add it so the menu matches.
        const opt = document.createElement("option");
        opt.value = hit;
        opt.textContent = hit;
        sel.appendChild(opt);
        sel.value = hit;
      }
    }
  };
  apply("theme", state.themeList, "theme");
  apply("layout", state.layoutList, "layout");
  apply("aspect", state.aspectList, "aspect");
}

/** Character offset where the markdown body starts (after closing `---`). */
function frontMatterBodyOffset(md) {
  const { end, lines } = parseFrontMatter(md);
  if (end < 0) return 0;
  let len = lines.slice(0, end + 1).join("\n").length;
  if (len < md.length && md[len] === "\n") len += 1;
  return len;
}

/** Write a control into front-matter and refresh the preview. */
function applyControlToMarkdown(key, value) {
  state[key] = value;
  state.applyingControls = true;
  try {
    const ta = $("editor");
    const pos = ta.selectionStart;
    const before = ta.value;
    const next = setFrontMatterField(before, key, value);
    if (next !== before) {
      const bodyAt = frontMatterBodyOffset(before);
      const delta = next.length - before.length;
      ta.value = next;
      // Caret in the body shifts by the FM edit size; caret inside FM stays put.
      const at =
        pos >= bodyAt
          ? Math.min(next.length, pos + delta)
          : Math.min(next.length, pos);
      try {
        ta.setSelectionRange(at, at);
      } catch {
        /* ignore */
      }
    }
    // Keep the matching <select> selected (no change event).
    const sel = $(key);
    if (sel && sel.value !== value) sel.value = value;
  } finally {
    state.applyingControls = false;
  }
  schedulePreview();
}

// ---------------------------------------------------------------------------
// In-place DOM morph (same idea as `md2any --serve --edit`)
// Mutates `from` to match `to` so the *same* element stays in the tree:
// host state (.caret), identity for the mounted map, and unchanged subtrees
// (images etc.) survive. Used to patch individual slides on each keystroke —
// we do *not* replace the whole <section> when content changes.
// ---------------------------------------------------------------------------
function imported(from, node) {
  return from.ownerDocument.importNode(node, true);
}
function morphAttrs(from, to) {
  for (let i = from.attributes.length - 1; i >= 0; i--) {
    const n = from.attributes[i].name;
    if (!to.hasAttribute(n)) from.removeAttribute(n);
  }
  for (let i = 0; i < to.attributes.length; i++) {
    const a = to.attributes[i];
    if (from.getAttribute(a.name) !== a.value) from.setAttribute(a.name, a.value);
  }
}
/**
 * Morph `from` into the shape of `to`. Returns the live element that remains
 * in the document (usually `from`; a new node only if the tag name changed).
 */
function morph(from, to) {
  if (from.nodeName !== to.nodeName) {
    const neu = imported(from, to);
    from.replaceWith(neu);
    return neu;
  }
  if (from.nodeType === 1) morphAttrs(from, to);
  let f = from.firstChild;
  let t = to.firstChild;
  while (t) {
    const nt = t.nextSibling;
    if (!f) {
      from.appendChild(imported(from, t));
      t = nt;
      continue;
    }
    const nf = f.nextSibling;
    if (f.nodeType !== t.nodeType || f.nodeName !== t.nodeName) {
      from.replaceChild(imported(from, t), f);
    } else if (f.nodeType === 3 || f.nodeType === 8) {
      if (f.nodeValue !== t.nodeValue) f.nodeValue = t.nodeValue;
    } else if (f.nodeType === 1) {
      morph(f, t);
    }
    f = nf;
    t = nt;
  }
  while (f) {
    const nf = f.nextSibling;
    from.removeChild(f);
    f = nf;
  }
  return from;
}

/** Snapshot scroll offsets for every element that might be the scrollport. */
function capturePreviewScroll(doc) {
  if (!doc) return [];
  const cands = [
    doc.scrollingElement,
    doc.documentElement,
    doc.body,
    doc.querySelector(".deck"),
    doc.querySelector("main"),
    doc.querySelector("#deck"),
  ].filter(Boolean);
  const seen = new Set();
  const out = [];
  for (const el of cands) {
    if (seen.has(el)) continue;
    seen.add(el);
    out.push({ el, top: el.scrollTop, left: el.scrollLeft });
  }
  return out;
}

function restorePreviewScroll(snapshots) {
  for (const s of snapshots) {
    if (!s.el || !s.el.isConnected) continue;
    s.el.scrollTop = s.top;
    s.el.scrollLeft = s.left;
  }
}

/** Body-relative line under the caret (front-matter stripped), matching HTML data-line. */
function caretBodyLine() {
  const ta = $("editor");
  const text = ta.value;
  const before = text.slice(0, ta.selectionStart);
  const absLine = before.split("\n").length - 1; // 0-based
  const lines = text.split("\n");
  let fm = 0;
  if (lines[0] && lines[0].trim() === "---") {
    for (let j = 1; j < lines.length; j++) {
      if (lines[j].trim() === "---") {
        fm = j + 1;
        break;
      }
    }
  }
  return Math.max(0, absLine - fm);
}

/** How many leading lines are front-matter (including both `---` fences). */
function frontMatterLineCount(md) {
  const { end } = parseFrontMatter(md);
  return end < 0 ? 0 : end + 1;
}

/** Character offset of the start of 0-based absolute line `absLine`. */
function lineStartOffset(text, absLine) {
  if (absLine <= 0) return 0;
  let pos = 0;
  let line = 0;
  while (line < absLine && pos < text.length) {
    const n = text.indexOf("\n", pos);
    if (n < 0) return text.length;
    pos = n + 1;
    line++;
  }
  return pos;
}

/**
 * Markdown [start, end) char range for a slide whose `data-line` is `bodyLine`.
 * End is the start of the next slide’s source (or EOF).
 */
function slideMarkdownRange(bodyLine, slideEls) {
  const text = $("editor").value;
  const fm = frontMatterLineCount(text);
  const absStart = fm + Math.max(0, bodyLine);

  let nextBody = null;
  const lines = (slideEls || [])
    .map((s) => Number(s.getAttribute("data-line") || 0))
    .filter((n) => !Number.isNaN(n));
  for (const l of lines) {
    if (l > bodyLine && (nextBody === null || l < nextBody)) nextBody = l;
  }
  // Outline can include slides the DOM hasn’t painted yet.
  for (const s of state.slides || []) {
    const l = Number(s.sourceLine);
    if (!Number.isNaN(l) && l > bodyLine && (nextBody === null || l < nextBody)) {
      nextBody = l;
    }
  }

  const absEndLine =
    nextBody != null ? fm + nextBody : text.split("\n").length;
  const start = lineStartOffset(text, absStart);
  let end =
    nextBody != null ? lineStartOffset(text, fm + nextBody) : text.length;
  // Prefer not to include the next slide’s leading blank-only separation? keep as-is.
  // Drop a single trailing newline from the selection so the flash hugs content.
  if (end > start && text[end - 1] === "\n") end -= 1;
  return {
    start,
    end: Math.max(start, end),
    absStart,
    absEndLine,
  };
}

function editorLineMetrics() {
  const ta = $("editor");
  const style = getComputedStyle(ta);
  let lh = parseFloat(style.lineHeight);
  if (!lh || Number.isNaN(lh)) {
    lh = parseFloat(style.fontSize) * 1.55 || 20;
  }
  const padTop = parseFloat(style.paddingTop) || 0;
  const padLeft = parseFloat(style.paddingLeft) || 0;
  return { ta, lh, padTop, padLeft };
}

/** Pulse a translucent band over the slide’s markdown + select the range briefly. */
function flashEditorRange(range) {
  const { ta, lh, padTop } = editorLineMetrics();
  const flash = $("editor-flash");
  if (!flash) return;

  // Scroll so the range is in view before we measure.
  const viewPad = lh * 2;
  const targetTop = range.absStart * lh - viewPad;
  ta.scrollTop = Math.max(0, targetTop);

  const lineCount = Math.max(1, range.absEndLine - range.absStart);
  flash.style.top = `${padTop + range.absStart * lh - ta.scrollTop}px`;
  flash.style.height = `${lineCount * lh}px`;
  flash.classList.remove("run");
  // Restart CSS animation.
  void flash.offsetWidth;
  flash.classList.add("run");

  ta.classList.add("flash-select");
  ta.focus();
  try {
    ta.setSelectionRange(range.start, range.end);
  } catch {
    /* ignore */
  }
  updateCaretInfo();

  clearTimeout(state.flashTimer);
  state.flashTimer = setTimeout(() => {
    ta.classList.remove("flash-select");
    flash.classList.remove("run");
    // Leave the caret at the start of the slide’s source.
    try {
      ta.setSelectionRange(range.start, range.start);
    } catch {
      /* ignore */
    }
    updateCaretInfo();
  }, 1100);

  // Keep the band pinned while the user isn’t scrolling; refresh once after scroll settle.
  clearTimeout(state.flashScrollTimer);
  const pin = () => {
    flash.style.top = `${padTop + range.absStart * lh - ta.scrollTop}px`;
  };
  ta.addEventListener("scroll", pin);
  state.flashScrollTimer = setTimeout(() => {
    ta.removeEventListener("scroll", pin);
  }, 1100);
}

/**
 * Preview → editor: halo the slide, move the caret, flash its markdown range.
 */
function selectSlideFromPreview(slideEl) {
  if (!slideEl) return;
  const doc = slideEl.ownerDocument;
  const slides = Array.from(doc.querySelectorAll(".slide"));
  slides.forEach((s) => s.classList.toggle("caret", s === slideEl));
  const idx = slides.indexOf(slideEl) + 1;
  state.activeSlide = idx;
  // Keep HTML window centered on the clicked slide.
  state.preview.focusIndex = Math.max(0, idx - 1);
  if (state.slides && state.slides.length) renderSlideList(state.slides);

  const bodyLine = Number(slideEl.getAttribute("data-line") || 0);
  const range = slideMarkdownRange(bodyLine, slides);
  flashEditorRange(range);
  // Ensure the window covers this slide (no black spacer).
  schedulePreview();
}

/** Wire click-on-slide inside the preview iframe (survives morph via delegation). */
function bindPreviewClicks() {
  const doc = $("preview").contentDocument;
  if (!doc) return;
  // Flag lives on the Document (not an element attribute), so DOM morph
  // does not clear it — but a fresh srcdoc document will re-bind cleanly.
  if (doc._md2anyClickBound) {
    ensurePreviewClickStyle(doc);
    return;
  }
  doc._md2anyClickBound = true;
  doc.addEventListener(
    "click",
    (e) => {
      const slide =
        e.target && e.target.closest ? e.target.closest(".slide") : null;
      if (!slide) return;
      e.preventDefault();
      selectSlideFromPreview(slide);
    },
    true
  );
  ensurePreviewClickStyle(doc);
}

function ensurePreviewClickStyle(doc) {
  if (doc.getElementById("md2any-click-style")) return;
  const st = doc.createElement("style");
  st.id = "md2any-click-style";
  st.textContent =
    "body.edit .slide{cursor:pointer;}body.edit .slide:hover{outline:1px solid color-mix(in srgb,var(--accent) 55%,transparent);outline-offset:2px;}";
  (doc.head || doc.documentElement).appendChild(st);
}

/**
 * Soft-highlight the slide under the caret. When `scroll` is true, bring it
 * into view (click / keyboard navigation / rail — not ordinary typing).
 * If the active slide is outside the mounted HTML window, request a recenter.
 */
function focusCaretSlide(scroll) {
  const iframe = $("preview");
  const doc = iframe.contentDocument;
  if (!doc) return;

  const active0 = activeSlideIndex0();
  const idx = active0 + 1;
  const changed = state.activeSlide !== idx;
  state.activeSlide = idx;
  if (changed && state.slides && state.slides.length) {
    renderSlideList(state.slides);
  }

  // Halo on mounted slides (only the window is in the DOM).
  // Only touch classList when the value actually changes — avoids style
  // thrashing when preview patches fire after every keystroke.
  const mounted = Array.from(doc.querySelectorAll(".slide"));
  let pick = null;
  for (const s of mounted) {
    const n = Number(s.getAttribute("data-slide") || 0);
    const on = n === idx;
    if (on) pick = s;
    if (s.classList.contains("caret") !== on) {
      s.classList.toggle("caret", on);
    }
  }

  // Active slide not in the mounted HTML window → recenter the window.
  if (
    state.slides.length > 0 &&
    !pick &&
    state.preview.lastTo != null &&
    (active0 < state.preview.lastFrom || active0 >= state.preview.lastTo)
  ) {
    schedulePreview();
    return;
  }

  if (scroll && pick) {
    pick.scrollIntoView({ block: "center", behavior: "smooth" });
  }
}

/** Keys that move the caret without editing — preview should follow. */
const NAV_KEYS = new Set([
  "ArrowUp",
  "ArrowDown",
  "ArrowLeft",
  "ArrowRight",
  "Home",
  "End",
  "PageUp",
  "PageDown",
]);

function saveLocal() {
  try {
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({
        md: $("editor").value,
        theme: state.theme,
        layout: state.layout,
        aspect: state.aspect,
        uiTheme: document.documentElement.getAttribute("data-theme"),
        assets: state.assets || {},
      })
    );
  } catch {
    /* quota / private mode */
  }
}

function loadLocal() {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    return JSON.parse(raw);
  } catch {
    return null;
  }
}

function assetsJson() {
  if (!state.assets || Object.keys(state.assets).length === 0) return null;
  return JSON.stringify(state.assets);
}

function setBadge(text, kind) {
  const el = $("status-badge");
  el.textContent = text;
  el.className = "badge" + (kind ? ` ${kind}` : "");
}

function countWords(s) {
  const t = s.trim();
  return t ? t.split(/\s+/).length : 0;
}

function fillSelect(sel, items, current) {
  sel.innerHTML = "";
  for (const name of items) {
    const opt = document.createElement("option");
    opt.value = name;
    opt.textContent = name;
    if (name === current) opt.selected = true;
    sel.appendChild(opt);
  }
}

function schedulePreview() {
  // Keep toolbar in lockstep with front-matter as the user types.
  syncControlsFromMarkdown();
  clearTimeout(state.previewTimer);
  state.previewTimer = setTimeout(runPreview, PREVIEW_MS);
  updateStats();
  saveLocal();
}

/** Mark that the user is typing — shrink virtual HTML window temporarily. */
function noteTyping() {
  state.typingUntil = Date.now() + TYPING_IDLE_MS;
}

function currentWindowRadius() {
  return Date.now() < state.typingUntil ? WINDOW_RADIUS_TYPING : WINDOW_RADIUS;
}

/** 0-based active slide from caret + current outline (sourceLine). */
function caretSlideIndex0() {
  const bl = caretBodyLine();
  const slides = state.slides || [];
  if (!slides.length) return 0;
  let pick = 0;
  for (let i = 0; i < slides.length; i++) {
    if (Number(slides[i].sourceLine) <= bl) pick = i;
    else break;
  }
  return pick;
}

/**
 * Center index for the HTML window: explicit focus (rail/scroll) wins,
 * otherwise the caret's slide.
 */
function activeSlideIndex0() {
  if (state.preview.focusIndex != null && state.slides.length) {
    return Math.max(
      0,
      Math.min(state.slides.length - 1, state.preview.focusIndex | 0)
    );
  }
  return caretSlideIndex0();
}

/**
 * HTML window around `center0`. Pins to the start/end of the deck so title
 * slides are never left as empty black spacers when you're near either end.
 */
function windowRange(center0, slideCount) {
  const n = Math.max(0, slideCount | 0);
  if (n === 0) return { from: 0, to: 0 };
  const R = currentWindowRadius();
  const span = R * 2 + 1;
  const c = Math.max(0, Math.min(n - 1, center0 | 0));
  // Near the start → always include slide 1.
  if (c <= R) {
    return { from: 0, to: Math.min(n, span) };
  }
  // Near the end → always include the last slide.
  if (c >= n - 1 - R) {
    return { from: Math.max(0, n - span), to: n };
  }
  return {
    from: Math.max(0, c - R),
    to: Math.min(n, c + R + 1),
  };
}

/** Estimate which slide is mid-viewport from spacer scroll (for scroll-loading). */
function slideIndexFromPreviewScroll(doc) {
  const se = doc.scrollingElement || doc.documentElement;
  if (!se) return activeSlideIndex0();
  const h = Math.max(1, state.preview.slideHeight || DEFAULT_SLIDE_BLOCK);
  const mid = se.scrollTop + (doc.defaultView?.innerHeight || se.clientHeight) * 0.35;
  const n = Math.max(1, state.slides.length || 1);
  return Math.max(0, Math.min(n - 1, Math.floor(mid / h)));
}

function bindPreviewScroll() {
  const doc = $("preview").contentDocument;
  if (!doc || doc._md2anyScrollBound) return;
  doc._md2anyScrollBound = true;
  const onScroll = () => {
    clearTimeout(state.preview.scrollTimer);
    state.preview.scrollTimer = setTimeout(() => {
      if (!state.slides.length) return;
      const idx = slideIndexFromPreviewScroll(doc);
      const { from, to } = windowRange(idx, state.slides.length);
      // Only re-fetch when the scroll position needs slides we don't have.
      if (
        state.preview.lastFrom == null ||
        idx < state.preview.lastFrom ||
        idx >= state.preview.lastTo ||
        from < state.preview.lastFrom ||
        to > state.preview.lastTo
      ) {
        state.preview.focusIndex = idx;
        schedulePreview();
      }
    }, 100);
  };
  doc.addEventListener("scroll", onScroll, { passive: true, capture: true });
}

async function ensurePreviewShell() {
  if (state.preview.shellReady) return;
  const iframe = $("preview");
  await new Promise((resolve) => {
    iframe.addEventListener("load", resolve, { once: true });
    iframe.srcdoc = `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width, initial-scale=1"/>
<style id="theme-css"></style>
<style id="session-fonts"></style>
</head>
<body class="edit">
<main class="deck virtual" id="deck">
  <div class="v-spacer" id="sp-top"></div>
  <div id="slide-window"></div>
  <div class="v-spacer" id="sp-bot"></div>
</main>
</body>
</html>`;
  });
  state.preview.shellReady = true;
  state.previewReady = true;
  bindPreviewClicks();
  bindPreviewScroll();
  // Apply any session fonts already loaded into OPFS / memory.
  if (typeof window.__studioPro?.injectSessionFonts === "function") {
    await window.__studioPro.injectSessionFonts();
  }
}

/** Persist last successful preview window outline + CSS to OPFS (best effort). */
async function cacheLastGoodPreview(win) {
  if (!win || typeof navigator === "undefined" || !navigator.storage?.getDirectory) {
    return;
  }
  try {
    const root = await navigator.storage.getDirectory();
    const dir = await root.getDirectoryHandle("md2any-studio", { create: true });
    const file = await dir.getFileHandle("last-good-preview.json", { create: true });
    const payload = {
      savedAt: Date.now(),
      title: win.title,
      slideCount: win.slideCount,
      bodyClass: win.bodyClass,
      css: win.css,
      structureKey: win.structureKey,
      htmlFrom: win.htmlFrom,
      htmlTo: win.htmlTo,
      // Keep outline only — HTML frags can be large; enough for rail recovery.
      slides: (win.slides || []).map((s) => ({
        index: s.index,
        title: s.title,
        kind: s.kind,
        sourceLine: s.sourceLine,
        hasNotes: s.hasNotes,
        notes: s.notes || null,
        html: s.html || null,
        contentKey: s.contentKey,
      })),
    };
    const w = await file.createWritable();
    await w.write(JSON.stringify(payload));
    await w.close();
  } catch {
    /* OPFS unavailable / private mode */
  }
}

async function loadLastGoodPreview() {
  if (typeof navigator === "undefined" || !navigator.storage?.getDirectory) {
    return null;
  }
  try {
    const root = await navigator.storage.getDirectory();
    const dir = await root.getDirectoryHandle("md2any-studio");
    const file = await dir.getFileHandle("last-good-preview.json");
    const blob = await (await file.getFile()).text();
    return JSON.parse(blob);
  } catch {
    return null;
  }
}

/**
 * Apply a virtualised preview window: outline for all slides, HTML only for
 * htmlFrom..htmlTo. Patches DOM incrementally when structure_key is stable.
 */
function applyPreviewWindow(win, opts = {}) {
  const doc = $("preview").contentDocument;
  if (!doc) return;

  const scrollY =
    opts.preserveScroll !== false
      ? doc.scrollingElement?.scrollTop ?? doc.documentElement.scrollTop
      : null;

  // Theme / layout CSS
  const cssEl = doc.getElementById("theme-css");
  if (cssEl && state.preview.cssKey !== win.structureKey) {
    // structureKey includes theme chrome; refresh CSS when it changes.
    cssEl.textContent = win.css || "";
  }
  if (win.bodyClass) doc.body.className = win.bodyClass;

  const n = win.slideCount | 0;
  const from = win.htmlFrom | 0;
  const to = win.htmlTo | 0;
  const h = state.preview.slideHeight;

  const spTop = doc.getElementById("sp-top");
  const spBot = doc.getElementById("sp-bot");
  const windowEl = doc.getElementById("slide-window");
  if (!windowEl) return;

  if (spTop) spTop.style.height = `${from * h}px`;
  if (spBot) spBot.style.height = `${Math.max(0, n - to) * h}px`;

  const structureChanged = state.preview.structureKey !== win.structureKey;
  if (structureChanged) {
    state.preview.structureKey = win.structureKey;
    state.preview.mounted.clear();
    windowEl.replaceChildren();
  }

  // Drop mounted slides outside the window.
  for (const idx of [...state.preview.mounted.keys()]) {
    if (idx < from || idx >= to) {
      const m = state.preview.mounted.get(idx);
      m?.el?.remove();
      state.preview.mounted.delete(idx);
    }
  }

  // Active slide (1-based data-slide) for restoring .caret after morph
  // (engine HTML never includes host-only classes like caret).
  const activeIdx = (opts.activeIndex0 != null ? opts.activeIndex0 : activeSlideIndex0()) + 1;

  // Upsert HTML for the window — **morph in place** when the slide node already
  // exists so we don't throw away the element (halo, identity) every keystroke.
  for (let i = from; i < to; i++) {
    const meta = win.slides[i];
    if (!meta || meta.html == null) continue;
    const prev = state.preview.mounted.get(i);
    if (prev && prev.key === meta.contentKey && prev.el.isConnected) {
      continue; // byte-identical HTML — leave the live node alone
    }

    const tmp = doc.createElement("div");
    tmp.innerHTML = String(meta.html).trim();
    const next = tmp.firstElementChild;
    if (!next) continue;

    const wantCaret =
      (prev?.el?.classList?.contains("caret") ?? false) || meta.index === activeIdx;

    let live;
    if (prev?.el?.isConnected && prev.el.nodeName === next.nodeName) {
      // Edit in place: same <section>, patched children/attrs only.
      live = morph(prev.el, next);
    } else if (prev?.el?.isConnected) {
      live = morph(prev.el, next);
    } else {
      live = next;
    }
    if (wantCaret) live.classList.add("caret");
    else live.classList.remove("caret");
    state.preview.mounted.set(i, { key: meta.contentKey, el: live });
  }

  // Ensure DOM order matches window order (only re-parents when needed).
  const ordered = [];
  for (let i = from; i < to; i++) {
    const m = state.preview.mounted.get(i);
    if (m?.el) ordered.push(m.el);
  }
  let needsReorder = windowEl.childNodes.length !== ordered.length;
  if (!needsReorder) {
    for (let i = 0; i < ordered.length; i++) {
      if (windowEl.childNodes[i] !== ordered[i]) {
        needsReorder = true;
        break;
      }
    }
  }
  if (needsReorder) windowEl.replaceChildren(...ordered);

  // Measure real slide height for spacers.
  const sample = windowEl.querySelector(".slide");
  if (sample) {
    const rect = sample.getBoundingClientRect();
    if (rect.height > 40) {
      state.preview.slideHeight = Math.round(rect.height + 20);
      // Re-apply spacers with measured height.
      if (spTop) spTop.style.height = `${from * state.preview.slideHeight}px`;
      if (spBot)
        spBot.style.height = `${Math.max(0, n - to) * state.preview.slideHeight}px`;
    }
  }

  if (scrollY != null && doc.scrollingElement) {
    doc.scrollingElement.scrollTop = scrollY;
  } else if (scrollY != null) {
    doc.documentElement.scrollTop = scrollY;
  }

  // Global outline (rail) — always full deck, no second engine pass.
  state.slides = (win.slides || []).map((s) => ({
    index: s.index,
    title: s.title,
    kind: s.kind,
    sourceLine: s.sourceLine,
    hasNotes: !!(s.hasNotes || (s.notes && String(s.notes).trim())),
    notes: s.notes || null,
  }));
  state.preview.lastFrom = from;
  state.preview.lastTo = to;
  state.preview.cssKey = win.structureKey;

  renderSlideList(state.slides);
  $("stat-slides").textContent = String(n);
  $("preview-meta").textContent =
    n === 0
      ? "empty"
      : `${n} slides · html ${from + 1}–${Math.max(from, to)}`;
  // OPFS last-good preview shell (structure + CSS) for cold recovery.
  cacheLastGoodPreview(win).catch(() => {});
  if (typeof window.__studioExtras?.onPreviewApplied === "function") {
    window.__studioExtras.onPreviewApplied();
  }
  // When typing ends, expand the HTML window once more for full neighbors.
  if (state.typingUntil && Date.now() < state.typingUntil) {
    clearTimeout(state._typingExpandTimer);
    state._typingExpandTimer = setTimeout(() => {
      if (Date.now() >= state.typingUntil) schedulePreview();
    }, TYPING_IDLE_MS + 20);
  }
}

async function runPreview() {
  if (!state.ready) return;
  syncControlsFromMarkdown();
  const gen = ++state.previewGen;
  const firstPaint = !state.preview.shellReady;
  if (firstPaint) $("preview-overlay").classList.add("show");

  try {
    await ensurePreviewShell();
    if (gen !== state.previewGen) return;

    const md = $("editor").value;
    // Cold start: always load from the beginning so title slides render.
    const active = state.slides.length
      ? activeSlideIndex0()
      : 0;
    const guessN = Math.max(
      state.slides.length,
      active + WINDOW_RADIUS * 2 + 1,
      WINDOW_RADIUS * 2 + 1
    );
    let { from, to } = windowRange(active, guessN);

    const req = {
      markdown: md,
      theme: state.theme,
      aspect: state.aspect,
      layout: state.layout,
      assetsJson: assetsJson(),
      htmlFrom: from,
      htmlTo: to,
    };

    let win = await engine.previewWindow(req);
    if (gen !== state.previewGen) return;

    // If slide count / focus implies a different window, re-fetch once.
    const n = win.slideCount | 0;
    state.slides = (win.slides || []).map((s) => ({
      index: s.index,
      title: s.title,
      kind: s.kind,
      sourceLine: s.sourceLine,
      hasNotes: !!(s.hasNotes || (s.notes && String(s.notes).trim())),
      notes: s.notes || null,
    }));
    const active2 = activeSlideIndex0();
    const w2 = windowRange(active2, n);
    if (w2.from !== win.htmlFrom || w2.to !== win.htmlTo) {
      win = await engine.previewWindow({
        ...req,
        htmlFrom: w2.from,
        htmlTo: w2.to,
      });
      if (gen !== state.previewGen) return;
    }

    const activeAfter = activeSlideIndex0();
    applyPreviewWindow(win, {
      preserveScroll: !firstPaint && state.preview.focusIndex == null,
      activeIndex0: activeAfter,
    });
    bindPreviewClicks();
    bindPreviewScroll();
    // Scroll into view when focus came from rail/caret navigation.
    if (firstPaint || state.preview.focusIndex != null) {
      focusCaretSlide(true);
    } else {
      focusCaretSlide(false);
    }
    setBadge("live", "ok");
  } catch (e) {
    console.error(e);
    setBadge("preview error", "err");
    $("preview-meta").textContent = String(e);
    // Best-effort: restore last-good window from OPFS if shell is empty.
    try {
      const cached = await loadLastGoodPreview();
      if (cached && cached.slides?.length && !state.preview.mounted.size) {
        await ensurePreviewShell();
        applyPreviewWindow(cached, { preserveScroll: false });
        setBadge("cached preview", "busy");
        $("preview-meta").textContent = "showing last-good OPFS cache";
      }
    } catch {
      /* ignore */
    }
  } finally {
    $("preview-overlay").classList.remove("show");
  }
}

function renderSlideList(slides) {
  const host = $("slide-list");
  host.innerHTML = "";
  for (const s of slides) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "slide-item" + (s.index === state.activeSlide ? " active" : "");
    btn.innerHTML = `<span class="n">${s.index}</span><span class="t"></span><span class="k"></span>`;
    btn.querySelector(".t").textContent = s.title || `Slide ${s.index}`;
    btn.querySelector(".k").textContent = s.kind || "content";
    btn.addEventListener("click", () => {
      state.activeSlide = s.index;
      state.preview.focusIndex = Math.max(0, (s.index | 0) - 1);
      renderSlideList(slides);
      // Jump editor + flash, then rebuild HTML window around this slide.
      const range = slideMarkdownRange(
        Number(s.sourceLine) || 0,
        $("preview").contentDocument?.querySelectorAll?.(".slide") || []
      );
      flashEditorRange(range);
      schedulePreview();
    });
    host.appendChild(btn);
  }
  if (typeof window.__studioExtras?.onRailRendered === "function") {
    window.__studioExtras.onRailRendered(host);
  }
}

function jumpToLine(line) {
  const ta = $("editor");
  const text = ta.value;
  let pos = 0;
  let cur = 1;
  while (cur < line && pos < text.length) {
    const n = text.indexOf("\n", pos);
    if (n < 0) break;
    pos = n + 1;
    cur++;
  }
  ta.focus();
  ta.setSelectionRange(pos, pos);
  // Approximate scroll
  const lineHeight = 20;
  ta.scrollTop = Math.max(0, (line - 3) * lineHeight);
  updateCaretInfo();
}

function updateCaretInfo() {
  const ta = $("editor");
  const pos = ta.selectionStart;
  const upto = ta.value.slice(0, pos);
  const line = upto.split("\n").length;
  const col = upto.length - upto.lastIndexOf("\n");
  $("caret-info").textContent = `Ln ${line}, Col ${col}`;
}

function updateStats() {
  $("stat-words").textContent = String(countWords($("editor").value));
}

function base64ToBlob(b64, contentType) {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return new Blob([bytes], { type: contentType || "application/octet-stream" });
}

async function exportFormat(fmt) {
  if (!state.ready || state.exporting) return;
  state.exporting = true;
  setBadge(`export ${fmt}…`, "busy");
  $("btn-export").disabled = true;
  try {
    await new Promise((r) => setTimeout(r, 20));
    syncControlsFromMarkdown();
    // Full-deck export — runs in the worker so the UI stays responsive.
    const result = await engine.convert({
      markdown: $("editor").value,
      format: fmt,
      theme: state.theme,
      aspect: state.aspect,
      layout: state.layout,
      assetsJson: assetsJson(),
    });
    const blob = base64ToBlob(result.base64, result.contentType);
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = result.filename || `document.${fmt}`;
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(a.href), 2000);
    setBadge(`exported ${fmt}`, "ok");
    $("stat-slides").textContent = String(result.slideCount ?? "—");
    if (typeof window.__studioExtras?.markExported === "function") {
      window.__studioExtras.markExported();
    } else {
      const stamp = document.getElementById("export-stamp");
      if (stamp) stamp.textContent = "exported";
    }
  } catch (e) {
    console.error(e);
    setBadge("export failed", "err");
    alert("Export failed:\n" + e);
  } finally {
    state.exporting = false;
    $("btn-export").disabled = false;
    $("export-menu").classList.remove("open");
  }
}

function wireUi() {
  $("editor").addEventListener("input", () => {
    // Typing: follow caret, don't pin an old rail/scroll focus.
    state.preview.focusIndex = null;
    noteTyping();
    schedulePreview();
  });

  // Click a line in the markdown → jump preview to that slide.
  // Also follows pure caret moves (arrows / Home / End / PageUp|Down).
  // Ordinary typing only updates the halo, so scroll position stays put.
  const followCaret = (scroll) => {
    updateCaretInfo();
    // Editor navigation owns the window center again.
    state.preview.focusIndex = null;
    if (state.previewReady) focusCaretSlide(scroll);
  };
  $("editor").addEventListener("click", () => followCaret(true));
  $("editor").addEventListener("keyup", (e) => {
    followCaret(NAV_KEYS.has(e.key));
  });
  // Mouse-up covers click-and-drag selection landing on a new line.
  $("editor").addEventListener("mouseup", () => followCaret(true));

  // Dropdown → front-matter → preview. Markdown edits flow the other way
  // via syncControlsFromMarkdown() inside schedulePreview.
  $("theme").addEventListener("change", (e) => {
    applyControlToMarkdown("theme", e.target.value);
  });
  $("layout").addEventListener("change", (e) => {
    applyControlToMarkdown("layout", e.target.value);
  });
  $("aspect").addEventListener("change", (e) => {
    applyControlToMarkdown("aspect", e.target.value);
  });

  $("btn-sample").addEventListener("click", () => {
    $("editor").value = SAMPLE;
    // New document: reset virtual window bookkeeping.
    state.preview.structureKey = null;
    state.preview.mounted.clear();
    state.slides = [];
    syncControlsFromMarkdown();
    schedulePreview();
  });

  $("btn-open").addEventListener("click", () => $("file-input").click());
  $("file-input").addEventListener("change", async (e) => {
    const file = e.target.files?.[0];
    if (!file) return;
    $("editor").value = await file.text();
    state.preview.structureKey = null;
    state.preview.mounted.clear();
    state.slides = [];
    syncControlsFromMarkdown();
    schedulePreview();
    e.target.value = "";
  });

  $("btn-theme-toggle").addEventListener("click", () => {
    const cur = document.documentElement.getAttribute("data-theme") || "dark";
    document.documentElement.setAttribute("data-theme", cur === "dark" ? "light" : "dark");
    saveLocal();
  });

  const menu = $("export-menu");
  $("btn-export").addEventListener("click", (e) => {
    e.stopPropagation();
    menu.classList.toggle("open");
  });
  menu.querySelectorAll("[data-fmt]").forEach((btn) => {
    btn.addEventListener("click", () => exportFormat(btn.getAttribute("data-fmt")));
  });
  document.addEventListener("click", () => menu.classList.remove("open"));

  // Save / export shortcuts are owned by studio-extras (⌘S save, ⌘⇧S PDF).
}

async function main() {
  wireUi();
  setBadge("loading engine…", "busy");

  const meta = await engine.init();
  const ver = meta?.version || "?";
  $("stat-engine").textContent = `md2any ${ver} · wasm worker`;

  const themes = meta?.themes || [
    "light",
    "dark",
    "corporate",
    "sepia",
    "contrast",
    "midnight",
    "terminal",
    "pastel",
  ];
  const layouts = meta?.layouts || ["clean", "studio", "frame", "bold"];
  state.themeList = Array.from(themes);
  state.layoutList = Array.from(layouts);

  const extras = installStudioExtras({
    $,
    state,
    engine,
    schedulePreview,
    SAMPLE,
    syncControlsFromMarkdown,
    applyControlToMarkdown,
    setFrontMatterField,
    parseFrontMatter,
    exportFormat,
    setBadge,
    saveLocal,
  });
  window.__studioExtras = extras;
  extras.wire();

  const pro = installStudioPro({
    $,
    state,
    engine,
    schedulePreview,
    exportFormat,
    setBadge,
    parseFrontMatter,
  });
  window.__studioPro = pro;
  pro.wire();

  // Chain pro hooks onto extras preview callbacks.
  const prevOnPreview = extras.onPreviewApplied;
  extras.onPreviewApplied = () => {
    if (typeof prevOnPreview === "function") prevOnPreview();
    pro.onPreviewApplied?.();
  };

  const fromShare = await extras.tryLoadShareHash();
  let fromSeed = false;
  if (!fromShare) {
    // CLI `md2any file.md --studio` exposes seed at /__studio_seed.md (?seed=1).
    try {
      const params = new URLSearchParams(location.search);
      if (params.has("seed")) {
        const res = await fetch("./__studio_seed.md", { cache: "no-store" });
        if (res.ok) {
          const text = await res.text();
          if (text && text.trim()) {
            extras.loadMarkdown(text);
            fromSeed = true;
            // Drop the query so refresh uses localStorage / editor state.
            history.replaceState(null, "", location.pathname + location.hash);
          }
        }
      }
    } catch {
      /* offline / no seed endpoint */
    }
  }
  const saved = loadLocal();
  if (saved?.uiTheme) {
    document.documentElement.setAttribute("data-theme", saved.uiTheme);
  }
  if (!fromShare && !fromSeed) {
    $("editor").value = saved?.md || SAMPLE;
  }

  // Prefer values from the document's front-matter; fall back to last session
  // then engine defaults.
  syncControlsFromMarkdown();
  if (!parseFrontMatter($("editor").value).fields.theme) {
    state.theme = saved?.theme || state.theme || "midnight";
  }
  if (!parseFrontMatter($("editor").value).fields.layout) {
    state.layout = saved?.layout || state.layout || "clean";
  }
  if (!parseFrontMatter($("editor").value).fields.aspect) {
    state.aspect = saved?.aspect || state.aspect || "16:9";
  }

  fillSelect($("theme"), state.themeList, state.theme);
  fillSelect($("layout"), state.layoutList, state.layout);
  // Ensure aspect options include anything the document declares.
  const aspectSel = $("aspect");
  for (const a of state.aspectList) {
    if (![...aspectSel.options].some((o) => o.value === a)) {
      const opt = document.createElement("option");
      opt.value = a;
      opt.textContent = a;
      aspectSel.appendChild(opt);
    }
  }
  aspectSel.value = state.aspect;

  // Restore assets map if present
  if (saved?.assets && typeof saved.assets === "object") {
    state.assets = saved.assets;
  }

  state.ready = true;
  $("boot").classList.add("hide");
  setBadge("ready", "ok");
  schedulePreview();
  updateCaretInfo();
  // Baseline for dirty-tracking once content is in the editor.
  if (typeof extras.markCleanBaseline === "function") {
    extras.markCleanBaseline();
  } else if (state.fs) {
    // hash will be set on first onPreviewApplied via savedHash null → not dirty
  }

  // PWA
  if ("serviceWorker" in navigator) {
    navigator.serviceWorker.register("./sw.js").catch(() => {});
  }
}

main().catch((e) => {
  console.error(e);
  $("boot").querySelector("h2").textContent = "Failed to load engine";
  $("boot").querySelector("p").textContent = String(e);
  setBadge("load failed", "err");
});
