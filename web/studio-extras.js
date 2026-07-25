/**
 * md2any Studio extras — palette, templates, assets, share, rail tools, panels.
 * Installed from app.js via installStudioExtras(api).
 */

const TEMPLATES = [
  {
    id: "welcome",
    title: "Studio welcome",
    blurb: "Default tour of formats and math",
    load: (api) => api.SAMPLE,
  },
  {
    id: "talk",
    title: "Conference talk",
    blurb: "Title · sections · bullets · closer",
    body: `---
title: Your Talk Title
author: Your Name
theme: midnight
layout: clean
aspect: 16:9
---

# Your Talk Title

Conference 2026

---

## Agenda

1. Problem
2. Approach
3. Results
4. Next steps

---

## The problem

- Pain point one
- Pain point two
- Why now?

---

## Approach

- Idea in one sentence
- Key mechanism
- Trade-offs

---

## Results

| Metric | Before | After |
|--------|-------:|------:|
| Latency | 120 ms | 18 ms |
| Size | 40 MB | 5 MB |

---

## Thanks

Questions?

@you · link.example
`,
  },
  {
    id: "handout",
    title: "A4 handout",
    blurb: "Portrait page for print / PDF",
    body: `---
title: One-pager
theme: corporate
layout: clean
aspect: a4
break_mode: off
---

# One-pager

**Summary.** Two sentences that fit on a flyer.

## Highlights

- Point A with a number
- Point B with a constraint
- Point C with a call to action

## Detail

Short paragraph. Keep it scannable. Export PDF for the board pack.
`,
  },
  {
    id: "math",
    title: "Rich math (SVG)",
    blurb: "Display math with front-matter math: svg",
    body: `---
title: Rich Math
subtitle: Native display math without TeX
author: md2any
theme: light
math: svg
math_scale: 1.0
math_block_align: center
math_max_height: 180
math_macros:
  '\\RR': '\\mathbb{R}'
---

# Rich Math

## Fractions, Roots, And Scripts

Inline math stays readable source in \`math: svg\` mode: $E = mc^2$.

$$
\\left(\\frac{-b \\pm \\sqrt{b^2 - 4ac}}{2a}\\right)^2
$$

## Matrices And Arrays

$$
\\left[
\\begin{array}{cc}
\\frac{1}{\\sqrt{x^2 + 1}} & y_i^2 \\\\
\\alpha + \\beta & \\binom{n}{k}
\\end{array}
\\right]
$$

## Cases

$$
f(x)=
\\begin{cases}
x^2 & x \\ge 0 \\\\
-x & x < 0
\\end{cases}
$$

## Macros

$$
\\forall x \\in \\RR^n,\\quad \\exists y \\in \\RR : y > \\lVert x \\rVert
$$
`,
  },
  {
    id: "code",
    title: "Code deep-dive",
    blurb: "Terminal theme + fenced code",
    body: `---
title: Shipping the binary
theme: terminal
layout: studio
aspect: 16:9
---

# Shipping the binary

## Build

\`\`\`bash
cargo build --release
strip target/release/md2any
\`\`\`

## Check

\`\`\`bash
./target/release/md2any deck.md --check
\`\`\`

---

## Ship

- One ~5 MB binary
- No Chromium
- No LaTeX
`,
  },
  {
    id: "blank",
    title: "Blank deck",
    blurb: "Minimal front-matter",
    body: `---
title: Untitled
theme: light
layout: clean
aspect: 16:9
---

# Untitled

First slide.
`,
  },
];

const FORMAT_TRUTH = [
  {
    fmt: "PPTX",
    keeps: "Editable slides, themes, transitions, notes, images, tables, code",
    notes: "Best for delivering and co-editing in PowerPoint / Keynote",
  },
  {
    fmt: "PDF",
    keeps: "Print-faithful pages, embedded fonts, full-page math layout",
    notes: "Use a math font (auto STIX when installed) for dense equations",
  },
  {
    fmt: "DOCX / ODT",
    keeps: "Flowing document, headings, lists, tables",
    notes: "Not a slide canvas — report/handout profile",
  },
  {
    fmt: "ODP",
    keeps: "Editable Impress deck, similar to PPTX",
    notes: "LibreOffice-native",
  },
  {
    fmt: "HTML",
    keeps: "Browser deck + studio preview fidelity",
    notes: "Same IR; SVG math when math: svg",
  },
];

function utf8ToB64Url(str) {
  const bytes = new TextEncoder().encode(str);
  let bin = "";
  bytes.forEach((b) => (bin += String.fromCharCode(b)));
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function b64UrlToUtf8(b64) {
  const pad = b64.length % 4 === 0 ? "" : "=".repeat(4 - (b64.length % 4));
  const norm = b64.replace(/-/g, "+").replace(/_/g, "/") + pad;
  const bin = atob(norm);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return new TextDecoder().decode(bytes);
}

export function installStudioExtras(api) {
  const {
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
  } = api;

  const ui = {
    palette: $("#palette"),
    paletteInput: $("#palette-input"),
    paletteList: $("#palette-list"),
    drawer: $("#drawer"),
    drawerBody: $("#drawer-body"),
    drawerTitle: $("#drawer-title"),
    mathBadge: $("#math-badge"),
    empty: $("#empty-gallery"),
    talk: $("#talk-mode"),
    toast: $("#toast"),
  };

  function toast(msg, ms = 2200) {
    if (!ui.toast) return;
    ui.toast.textContent = msg;
    ui.toast.classList.add("show");
    clearTimeout(ui.toast._t);
    ui.toast._t = setTimeout(() => ui.toast.classList.remove("show"), ms);
  }

  function loadMarkdown(md, { resetPreview = true, clean = false } = {}) {
    const ta = $("editor");
    ta.value = md;
    if (resetPreview) {
      state.preview.structureKey = null;
      state.preview.mounted.clear();
      state.preview.focusIndex = null;
      state.slides = [];
    }
    syncControlsFromMarkdown();
    schedulePreview();
    updateMathBadge();
    updateReviewChips();
    maybeShowEmpty();
    saveLocal();
    if (clean) markClean();
    else markDirty();
  }

  function insertAtCaret(text) {
    const ta = $("editor");
    const start = ta.selectionStart;
    const end = ta.selectionEnd;
    const v = ta.value;
    ta.value = v.slice(0, start) + text + v.slice(end);
    const pos = start + text.length;
    ta.focus();
    ta.setSelectionRange(pos, pos);
    state.preview.focusIndex = null;
    schedulePreview();
  }

  function updateMathBadge() {
    if (!ui.mathBadge) return;
    const { fields } = parseFrontMatter($("editor").value);
    const mode = (fields.math || "unicode").toLowerCase();
    ui.mathBadge.textContent = `math: ${mode}`;
    ui.mathBadge.title = "Click to cycle math mode (unicode → svg → source)";
    ui.mathBadge.dataset.mode = mode;
  }

  function cycleMathMode() {
    const order = ["unicode", "svg", "source"];
    const { fields } = parseFrontMatter($("editor").value);
    const cur = (fields.math || "unicode").toLowerCase();
    const next = order[(order.indexOf(cur) + 1) % order.length];
    applyControlToMarkdown("math", next);
    updateMathBadge();
    toast(`Math mode → ${next}`);
  }

  function updateReviewChips() {
    const host = $("#review-chips");
    if (!host) return;
    const md = $("editor").value;
    const re = /<!--\s*@review:\s*([\s\S]*?)-->/gi;
    const items = [];
    let m;
    while ((m = re.exec(md))) {
      items.push(m[1].trim().replace(/\s+/g, " "));
    }
    host.innerHTML = "";
    if (!items.length) {
      host.hidden = true;
      return;
    }
    host.hidden = false;
    items.slice(0, 12).forEach((t, i) => {
      const chip = document.createElement("button");
      chip.type = "button";
      chip.className = "chip review";
      chip.textContent = t.length > 48 ? t.slice(0, 46) + "…" : t;
      chip.title = t;
      chip.addEventListener("click", () => {
        const idx = md.indexOf(items[i]);
        if (idx >= 0) {
          const ta = $("editor");
          ta.focus();
          ta.setSelectionRange(idx, idx + items[i].length);
        }
      });
      host.appendChild(chip);
    });
  }

  function maybeShowEmpty() {
    if (!ui.empty) return;
    const v = $("editor").value.trim();
    ui.empty.hidden = v.length > 0;
  }

  // ----- Templates gallery -----
  function renderGallery() {
    if (!ui.empty) return;
    const grid = ui.empty.querySelector(".gallery-grid");
    if (!grid) return;
    grid.innerHTML = "";
    TEMPLATES.forEach((t) => {
      const card = document.createElement("button");
      card.type = "button";
      card.className = "gallery-card";
      card.innerHTML = `<strong></strong><span></span>`;
      card.querySelector("strong").textContent = t.title;
      card.querySelector("span").textContent = t.blurb;
      card.addEventListener("click", () => {
        const body = t.load ? t.load(api) : t.body;
        loadMarkdown(body);
        toast(`Loaded “${t.title}”`);
      });
      grid.appendChild(card);
    });
  }

  // ----- Command palette -----
  let paletteItems = [];
  let paletteSel = 0;

  function buildPaletteCommands() {
    const cmds = [
      {
        id: "export-pdf",
        label: "Export PDF",
        hint: "⌘S",
        run: () => exportFormat("pdf"),
      },
      {
        id: "export-pptx",
        label: "Export PowerPoint",
        hint: "⌘⇧E",
        run: () => exportFormat("pptx"),
      },
      {
        id: "export-docx",
        label: "Export Word",
        run: () => exportFormat("docx"),
      },
      {
        id: "recipe-board",
        label: "Recipe: Board pack (PDF + PPTX)",
        run: () => recipeBoardPack(),
      },
      {
        id: "recipe-all",
        label: "Recipe: All formats",
        run: () => recipeAllFormats(),
      },
      {
        id: "talk",
        label: "Talk mode (fullscreen preview)",
        hint: "T",
        run: () => toggleTalkMode(true),
      },
      {
        id: "math-cycle",
        label: "Cycle math mode",
        run: () => cycleMathMode(),
      },
      {
        id: "insert-break",
        label: "Insert slide break (---)",
        run: () => insertAtCaret("\n\n---\n\n"),
      },
      {
        id: "insert-review",
        label: "Insert review comment",
        run: () => insertAtCaret("\n<!-- @review:  -->\n"),
      },
      {
        id: "share",
        label: "Copy share snapshot link",
        run: () => shareSnapshot(),
      },
      {
        id: "save",
        label: "Save markdown",
        hint: "⌘S",
        run: () => saveMarkdownFile(),
      },
      {
        id: "save-as",
        label: "Save markdown as…",
        run: () => saveMarkdownFile({ saveAs: true }),
      },
      {
        id: "open-file",
        label: "Open markdown file…",
        run: () => openMarkdownFile(),
      },
      {
        id: "open-folder",
        label: "Open folder…",
        run: () => openFolder(),
      },
      {
        id: "brand",
        label: "Import brand from .potx / .pptx",
        run: () => $("#brand-input")?.click(),
      },
      {
        id: "lint",
        label: "Stress deck (lint)",
        run: () => runLintPanel(),
      },
      {
        id: "ir",
        label: "IR inspector",
        run: () => showIrPanel(),
      },
      {
        id: "truth",
        label: "Format truth panel",
        run: () => showTruthPanel(),
      },
      {
        id: "help",
        label: "Keyboard shortcuts / help",
        hint: "?",
        run: () => showHelpPanel(),
      },
      {
        id: "macros",
        label: "Macro palette",
        run: () => showMacroPanel(),
      },
      {
        id: "theme-ui",
        label: "Toggle UI light/dark",
        run: () => $("btn-theme-toggle")?.click(),
      },
      ...state.themeList.map((t) => ({
        id: `theme-${t}`,
        label: `Theme → ${t}`,
        run: () => applyControlToMarkdown("theme", t),
      })),
      ...state.layoutList.map((l) => ({
        id: `layout-${l}`,
        label: `Layout → ${l}`,
        run: () => applyControlToMarkdown("layout", l),
      })),
      ...TEMPLATES.map((t) => ({
        id: `tpl-${t.id}`,
        label: `Template: ${t.title}`,
        run: () => {
          loadMarkdown(t.load ? t.load(api) : t.body);
          toast(`Loaded “${t.title}”`);
        },
      })),
    ];
    // Pro features (ghost / AI / git) register via window.__studioProCommands.
    if (typeof window.__studioProCommands === "function") {
      try {
        cmds.push(...window.__studioProCommands());
      } catch {
        /* ignore */
      }
    }
    return cmds;
  }

  function openPalette(seed = "") {
    if (!ui.palette) return;
    paletteItems = buildPaletteCommands();
    paletteSel = 0;
    ui.palette.hidden = false;
    ui.paletteInput.value = seed;
    renderPaletteList();
    ui.paletteInput.focus();
    ui.paletteInput.select();
  }

  function closePalette() {
    if (ui.palette) ui.palette.hidden = true;
  }

  function renderPaletteList() {
    const q = (ui.paletteInput.value || "").trim().toLowerCase();
    const filtered = !q
      ? paletteItems
      : paletteItems.filter(
          (c) =>
            c.label.toLowerCase().includes(q) ||
            (c.id && c.id.toLowerCase().includes(q))
        );
    ui.paletteList.innerHTML = "";
    filtered.slice(0, 40).forEach((c, i) => {
      const row = document.createElement("button");
      row.type = "button";
      row.className = "palette-row" + (i === paletteSel ? " active" : "");
      row.innerHTML = `<span class="plabel"></span><span class="phint"></span>`;
      row.querySelector(".plabel").textContent = c.label;
      row.querySelector(".phint").textContent = c.hint || "";
      row.addEventListener("click", () => {
        closePalette();
        c.run();
      });
      ui.paletteList.appendChild(row);
    });
    ui.paletteList._items = filtered;
  }

  function paletteMove(delta) {
    const items = ui.paletteList._items || [];
    if (!items.length) return;
    paletteSel = (paletteSel + delta + items.length) % items.length;
    renderPaletteList();
    const row = ui.paletteList.children[paletteSel];
    row?.scrollIntoView({ block: "nearest" });
  }

  function paletteConfirm() {
    const items = ui.paletteList._items || [];
    const c = items[paletteSel];
    if (!c) return;
    closePalette();
    c.run();
  }

  // ----- Drawer panels -----
  function openDrawer(title, htmlOrNode) {
    if (!ui.drawer) return;
    ui.drawer.hidden = false;
    ui.drawerTitle.textContent = title;
    ui.drawerBody.innerHTML = "";
    if (typeof htmlOrNode === "string") ui.drawerBody.innerHTML = htmlOrNode;
    else if (htmlOrNode) ui.drawerBody.appendChild(htmlOrNode);
  }

  function closeDrawer() {
    if (ui.drawer) ui.drawer.hidden = true;
  }

  function showTruthPanel() {
    const wrap = document.createElement("div");
    wrap.className = "truth-list";
    FORMAT_TRUTH.forEach((row) => {
      const el = document.createElement("div");
      el.className = "truth-row";
      el.innerHTML = `<h4></h4><p class="keeps"></p><p class="notes"></p>`;
      el.querySelector("h4").textContent = row.fmt;
      el.querySelector(".keeps").textContent = row.keeps;
      el.querySelector(".notes").textContent = row.notes;
      wrap.appendChild(el);
    });
    openDrawer("What each export keeps", wrap);
  }

  function showHelpPanel() {
    const wrap = document.createElement("div");
    wrap.className = "truth-list";
    wrap.innerHTML = `
      <div class="truth-row"><h4>⌘/Ctrl+K</h4><p class="keeps">Command palette</p></div>
      <div class="truth-row"><h4>⌘/Ctrl+S</h4><p class="keeps">Save markdown (File System Access or download)</p></div>
      <div class="truth-row"><h4>⌘/Ctrl+Shift+S</h4><p class="keeps">Export PDF</p></div>
      <div class="truth-row"><h4>⌘/Ctrl+Shift+E</h4><p class="keeps">Export PowerPoint</p></div>
      <div class="truth-row"><h4>T</h4><p class="keeps">Talk mode (fullscreen preview)</p><p class="notes">When focus is not in the editor</p></div>
      <div class="truth-row"><h4>Esc</h4><p class="keeps">Close palette, drawer, talk mode</p></div>
      <div class="truth-row"><h4>Rail / filmstrip</h4><p class="keeps">Drag rail to reorder · filmstrip scrub under preview</p></div>
      <div class="truth-row"><h4>Folder</h4><p class="keeps">Open project folder + load assets/</p></div>
      <div class="truth-row"><h4>Brand</h4><p class="keeps">Import .potx/.pptx colour+font scheme → style:</p></div>
      <div class="truth-row"><h4>Notes / fonts / ghost</h4><p class="keeps">Notes panel · OPFS fonts · Ghost HTML or SVG layout</p></div>
      <div class="truth-row"><h4>Ghost</h4><p class="keeps">Export HTML side-by-side (fidelity check)</p></div>
      <div class="truth-row"><h4>AI</h4><p class="keeps">BYO key surgical edits (localStorage)</p></div>
      <div class="truth-row"><h4>Git</h4><p class="keeps">Commit message helper · CLI host when seeded</p></div>
      <div class="truth-row"><h4>Share</h4><p class="keeps">URL fragment snapshot — never uploaded</p></div>
      <div class="truth-row"><h4>Math badge</h4><p class="keeps">Cycle unicode → svg → source in front-matter</p></div>
    `;
    openDrawer("Studio help", wrap);
  }

  function showIrPanel() {
    const data = {
      slideCount: state.slides.length,
      theme: state.theme,
      layout: state.layout,
      aspect: state.aspect,
      window: { from: state.preview.lastFrom, to: state.preview.lastTo },
      slides: state.slides,
      assets: Object.keys(state.assets || {}),
    };
    const pre = document.createElement("pre");
    pre.className = "ir-pre";
    pre.textContent = JSON.stringify(data, null, 2);
    openDrawer("IR inspector", pre);
  }

  function showMacroPanel() {
    const { fields } = parseFrontMatter($("editor").value);
    // parse math_macros is multi-line yaml — simple scan
    const md = $("editor").value;
    const macros = [];
    const block = md.match(/math_macros:\s*\n((?:[ \t]+.+\n?)*)/i);
    if (block) {
      block[1].split("\n").forEach((line) => {
        const m = line.match(/^\s*['"]?(\\?[^'":]+)['"]?\s*:\s*['"]?(.+?)['"]?\s*$/);
        if (m) macros.push({ key: m[1].replace(/^['"]|['"]$/g, ""), val: m[2] });
      });
    }
    const wrap = document.createElement("div");
    wrap.className = "macro-list";
    if (!macros.length) {
      wrap.innerHTML =
        "<p class='muted'>No <code>math_macros:</code> in front-matter. Add e.g. <code>'\\RR': '\\mathbb{R}'</code>.</p>";
    } else {
      macros.forEach((m) => {
        const b = document.createElement("button");
        b.type = "button";
        b.className = "chip";
        b.textContent = m.key;
        b.title = m.val;
        b.addEventListener("click", () => {
          insertAtCaret(m.key);
          toast(`Inserted ${m.key}`);
        });
        wrap.appendChild(b);
      });
    }
    openDrawer("Math macros", wrap);
  }

  async function runLintPanel() {
    setBadge("linting…", "busy");
    try {
      const hits = await engine.lint({
        markdown: $("editor").value,
        theme: state.theme,
        aspect: state.aspect,
        layout: state.layout,
        assetsJson:
          state.assets && Object.keys(state.assets).length
            ? JSON.stringify(state.assets)
            : null,
      });
      const wrap = document.createElement("div");
      wrap.className = "lint-list";
      if (!hits || !hits.length) {
        wrap.innerHTML = "<p class='ok'>No issues. Deck looks clean.</p>";
        toast("Lint clean");
      } else {
        hits.forEach((h) => {
          const row = document.createElement("button");
          row.type = "button";
          row.className = "lint-row";
          row.innerHTML = `<span class="ln">S${h.slide}</span><span class="lk"></span><span class="ld"></span>`;
          row.querySelector(".lk").textContent = h.kind;
          row.querySelector(".ld").textContent = h.detail;
          row.addEventListener("click", () => {
            state.preview.focusIndex = Math.max(0, (h.slide | 0) - 1);
            schedulePreview();
          });
          wrap.appendChild(row);
        });
        toast(`${hits.length} lint finding(s)`);
      }
      openDrawer("Stress deck (lint)", wrap);
      setBadge("live", "ok");
    } catch (e) {
      setBadge("lint failed", "err");
      toast(String(e));
    }
  }

  // ----- Share -----
  async function shareSnapshot() {
    try {
      const payload = {
        v: 1,
        md: $("editor").value,
        assets: state.assets || {},
      };
      const raw = JSON.stringify(payload);
      let encoded;
      if (globalThis.CompressionStream) {
        const cs = new CompressionStream("gzip");
        const stream = new Blob([raw]).stream().pipeThrough(cs);
        const buf = await new Response(stream).arrayBuffer();
        const bytes = new Uint8Array(buf);
        let bin = "";
        bytes.forEach((b) => (bin += String.fromCharCode(b)));
        encoded = "g1." + btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
      } else {
        encoded = "1." + utf8ToB64Url(raw);
      }
      if (encoded.length > 800_000) {
        toast("Document too large for URL share — download instead");
        return;
      }
      const url = `${location.origin}${location.pathname}#md2any.${encoded}`;
      await navigator.clipboard.writeText(url);
      toast("Share link copied (fragment stays local)");
    } catch (e) {
      toast("Share failed: " + e);
    }
  }

  async function tryLoadShareHash() {
    const hash = location.hash || "";
    const m = hash.match(/^#md2any\.(g1|1)\.(.+)$/);
    if (!m) return false;
    try {
      let json;
      if (m[1] === "g1" && globalThis.DecompressionStream) {
        const pad = m[2].length % 4 === 0 ? "" : "=".repeat(4 - (m[2].length % 4));
        const bin = atob(m[2].replace(/-/g, "+").replace(/_/g, "/") + pad);
        const bytes = new Uint8Array(bin.length);
        for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
        const ds = new DecompressionStream("gzip");
        const stream = new Blob([bytes]).stream().pipeThrough(ds);
        json = await new Response(stream).text();
      } else {
        json = b64UrlToUtf8(m[2]);
      }
      const data = JSON.parse(json);
      if (data.md) {
        if (data.assets && typeof data.assets === "object") {
          state.assets = data.assets;
        }
        loadMarkdown(data.md);
        toast("Loaded shared snapshot");
        history.replaceState(null, "", location.pathname + location.search);
        return true;
      }
    } catch (e) {
      console.warn("share hash", e);
      toast("Could not load share link");
    }
    return false;
  }

  // ----- Export recipes -----
  async function recipeBoardPack() {
    toast("Exporting board pack…");
    await exportFormat("pdf");
    await exportFormat("pptx");
    toast("Board pack: PDF + PPTX downloaded");
  }

  async function recipeAllFormats() {
    toast("Exporting all formats…");
    for (const f of ["pptx", "pdf", "docx", "odp", "odt", "html"]) {
      await exportFormat(f);
    }
    toast("All formats downloaded");
  }

  // ----- Talk mode (fullscreen live preview) -----
  let talkReturnParent = null;
  let talkReturnNext = null;

  function toggleTalkMode(on) {
    if (!ui.talk) return;
    const enable = on ?? ui.talk.hidden;
    const iframe = $("preview");
    const host = ui.talk.querySelector(".talk-frame");
    ui.talk.hidden = !enable;
    document.body.classList.toggle("talk", enable);

    if (enable) {
      if (iframe && host && !host.contains(iframe)) {
        talkReturnParent = iframe.parentElement;
        talkReturnNext = iframe.nextSibling;
        host.innerHTML = "";
        host.appendChild(iframe);
        iframe.classList.add("talk-iframe");
      }
      // Prefer Fullscreen API when available; overlay still works without it.
      const root = ui.talk;
      if (root.requestFullscreen) {
        root.requestFullscreen().catch(() => {});
      }
      toast("Talk mode — Esc to exit");
    } else {
      if (document.fullscreenElement) {
        document.exitFullscreen?.().catch(() => {});
      }
      if (iframe && talkReturnParent) {
        iframe.classList.remove("talk-iframe");
        if (talkReturnNext && talkReturnNext.parentNode === talkReturnParent) {
          talkReturnParent.insertBefore(iframe, talkReturnNext);
        } else {
          talkReturnParent.appendChild(iframe);
        }
        talkReturnParent = null;
        talkReturnNext = null;
      }
    }
  }

  document.addEventListener("fullscreenchange", () => {
    if (!document.fullscreenElement && ui.talk && !ui.talk.hidden) {
      toggleTalkMode(false);
    }
  });

  // ----- Assets: drop / paste -----
  function registerAsset(name, base64, mime) {
    let path = name.replace(/[^\w.\-]+/g, "-");
    if (!path.includes(".")) {
      const ext =
        mime === "image/jpeg" ? "jpg" : mime === "image/webp" ? "webp" : "png";
      path = `${path}.${ext}`;
    }
    if (!path.startsWith("assets/")) path = `assets/${path}`;
    // unique
    let final = path;
    let n = 1;
    while (state.assets[final]) {
      const i = path.lastIndexOf(".");
      final =
        i > 0
          ? `${path.slice(0, i)}-${n}${path.slice(i)}`
          : `${path}-${n}`;
      n++;
    }
    state.assets[final] = base64;
    return final;
  }

  function fileToAsset(file) {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => {
        const dataUrl = String(reader.result || "");
        const b64 = dataUrl.split(",")[1] || "";
        const path = registerAsset(file.name || "image", b64, file.type);
        resolve(path);
      };
      reader.onerror = reject;
      reader.readAsDataURL(file);
    });
  }

  async function handleImageFiles(files) {
    const list = [...files].filter((f) => f.type.startsWith("image/"));
    if (!list.length) return;
    const paths = [];
    for (const f of list) {
      paths.push(await fileToAsset(f));
    }
    const md = paths.map((p) => `![${p.split("/").pop()}](${p})`).join("\n\n");
    insertAtCaret((md.startsWith("\n") ? "" : "\n") + md + "\n");
    toast(`Added ${paths.length} image(s)`);
  }

  function tsvToMarkdownTable(text) {
    const rows = text
      .trim()
      .split(/\r?\n/)
      .map((r) => r.split("\t"));
    if (rows.length < 1) return null;
    const cols = Math.max(...rows.map((r) => r.length));
    const norm = rows.map((r) => {
      const x = r.slice();
      while (x.length < cols) x.push("");
      return x;
    });
    const header = norm[0];
    const sep = header.map(() => "---");
    const lines = [
      "| " + header.join(" | ") + " |",
      "| " + sep.join(" | ") + " |",
      ...norm.slice(1).map((r) => "| " + r.join(" | ") + " |"),
    ];
    return lines.join("\n");
  }

  // ----- Slide rail reorder -----
  function splitBodySections(md) {
    const { end, lines } = parseFrontMatter(md);
    let bodyLines;
    let fmText = "";
    if (end >= 0) {
      fmText = lines.slice(0, end + 1).join("\n");
      if (md.length > fmText.length && md[fmText.length] === "\n") {
        // body follows
      }
      bodyLines = lines.slice(end + 1);
    } else {
      bodyLines = lines;
    }
    const body = bodyLines.join("\n");
    // Split on slide breaks: a line that is exactly ---
    const sections = [];
    let cur = [];
    for (const line of bodyLines) {
      if (line.trim() === "---") {
        sections.push(cur.join("\n"));
        cur = [];
      } else {
        cur.push(line);
      }
    }
    sections.push(cur.join("\n"));
    return { fmText, sections, body };
  }

  function joinBodySections(fmText, sections) {
    const body = sections.join("\n---\n");
    if (!fmText) return body.replace(/^\n+/, "");
    return fmText + "\n" + body.replace(/^\n+/, "");
  }

  function reorderSections(fromIdx, toIdx) {
    const md = $("editor").value;
    const { fmText, sections } = splitBodySections(md);
    if (
      fromIdx < 0 ||
      toIdx < 0 ||
      fromIdx >= sections.length ||
      toIdx >= sections.length ||
      fromIdx === toIdx
    ) {
      return;
    }
    const next = sections.slice();
    const [item] = next.splice(fromIdx, 1);
    next.splice(toIdx, 0, item);
    loadMarkdown(joinBodySections(fmText, next), { resetPreview: true });
    toast(`Moved slide ${fromIdx + 1} → ${toIdx + 1}`);
  }

  // Patch renderSlideList after app defines it — we wrap via api.hookRail
  function enhanceRailList(host) {
    if (!host) return;
    let dragFrom = null;
    host.querySelectorAll(".slide-item").forEach((btn, i) => {
      btn.draggable = true;
      btn.addEventListener("dragstart", (e) => {
        dragFrom = i;
        e.dataTransfer.effectAllowed = "move";
        btn.classList.add("dragging");
      });
      btn.addEventListener("dragend", () => btn.classList.remove("dragging"));
      btn.addEventListener("dragover", (e) => {
        e.preventDefault();
        e.dataTransfer.dropEffect = "move";
      });
      btn.addEventListener("drop", (e) => {
        e.preventDefault();
        if (dragFrom == null || dragFrom === i) return;
        reorderSections(dragFrom, i);
        dragFrom = null;
      });
      // Double-click to rename first heading in that section
      btn.addEventListener("dblclick", (e) => {
        e.stopPropagation();
        const title = prompt("Slide title", state.slides[i]?.title || "");
        if (title == null || !title.trim()) return;
        renameSlideSection(i, title.trim());
      });
    });
  }

  function renameSlideSection(sectionIdx, newTitle) {
    const md = $("editor").value;
    const { fmText, sections } = splitBodySections(md);
    if (sectionIdx < 0 || sectionIdx >= sections.length) return;
    let sec = sections[sectionIdx];
    if (/^#+\s+/m.test(sec)) {
      sec = sec.replace(/^#+\s+.*$/m, (line) => {
        const hashes = line.match(/^#+/)[0];
        return `${hashes} ${newTitle}`;
      });
    } else {
      sec = `# ${newTitle}\n\n` + sec.replace(/^\n+/, "");
    }
    const next = sections.slice();
    next[sectionIdx] = sec;
    loadMarkdown(joinBodySections(fmText, next), { resetPreview: true });
    toast("Title updated");
  }

  // ----- File System Access (open / save / folder) -----
  // Handles live in state so app.js can also read them if needed.
  if (!state.fs) {
    state.fs = {
      fileHandle: null,
      dirHandle: null,
      fileName: null,
      dirty: false,
      savedHash: null,
    };
  }

  function supportsFsa() {
    return typeof window.showOpenFilePicker === "function";
  }

  function updateFileLabel() {
    const el = $("#file-label");
    if (!el) return;
    const name = state.fs.fileName || "unsaved";
    const dirty = state.fs.dirty ? " · modified" : "";
    const folder = state.fs.dirHandle ? " · folder" : "";
    el.textContent = name + dirty + folder;
    el.title = state.fs.fileHandle
      ? "Bound to local file (File System Access)"
      : "Not bound to a disk file — Save will download or pick a path";
  }

  function markDirty() {
    if (!state.fs.savedHash) {
      // Baseline not set yet (first paint) — don't scream "modified".
      state.fs.dirty = false;
      updateFileLabel();
      return;
    }
    const h = simpleHash($("editor").value);
    state.fs.dirty = h !== state.fs.savedHash;
    updateFileLabel();
  }

  function markClean() {
    state.fs.savedHash = simpleHash($("editor").value);
    state.fs.dirty = false;
    updateFileLabel();
  }

  function markCleanBaseline() {
    markClean();
  }

  async function openMarkdownFile() {
    try {
      if (supportsFsa()) {
        const [handle] = await window.showOpenFilePicker({
          multiple: false,
          types: [
            {
              description: "Markdown",
              accept: {
                "text/markdown": [".md", ".markdown"],
                "text/plain": [".txt", ".md"],
              },
            },
          ],
        });
        const file = await handle.getFile();
        const text = await file.text();
        state.fs.fileHandle = handle;
        state.fs.fileName = handle.name || file.name;
        loadMarkdown(text, { clean: true });
        toast(`Opened ${state.fs.fileName}`);
        return;
      }
    } catch (e) {
      if (e && e.name === "AbortError") return;
      console.warn("FSA open failed, falling back", e);
    }
    $("file-input")?.click();
  }

  async function saveMarkdownFile({ saveAs = false } = {}) {
    const md = $("editor").value;
    try {
      let handle = !saveAs ? state.fs.fileHandle : null;
      if (!handle && typeof window.showSaveFilePicker === "function") {
        handle = await window.showSaveFilePicker({
          suggestedName: state.fs.fileName || "deck.md",
          types: [
            {
              description: "Markdown",
              accept: { "text/markdown": [".md"] },
            },
          ],
        });
        state.fs.fileHandle = handle;
        state.fs.fileName = handle.name || "deck.md";
      }
      if (handle) {
        const writable = await handle.createWritable();
        await writable.write(md);
        await writable.close();
        markClean();
        toast(`Saved ${state.fs.fileName}`);
        // Best-effort: write virtual assets into open folder
        if (state.fs.dirHandle) {
          await writeAssetsToFolder().catch(() => {});
        }
        return;
      }
    } catch (e) {
      if (e && e.name === "AbortError") return;
      console.warn("FSA save failed, falling back", e);
    }
    // Download fallback
    const name = state.fs.fileName || "deck.md";
    const blob = new Blob([md], { type: "text/markdown;charset=utf-8" });
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = name;
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(a.href), 2000);
    state.fs.fileName = name;
    markClean();
    toast(`Downloaded ${name}`);
  }

  async function openFolder() {
    if (typeof window.showDirectoryPicker !== "function") {
      toast("Folder open needs Chrome/Edge File System Access");
      return;
    }
    try {
      const dir = await window.showDirectoryPicker({ mode: "readwrite" });
      state.fs.dirHandle = dir;
      // Prefer deck.md / README.md / first .md
      const mdFiles = [];
      for await (const [name, handle] of dir.entries()) {
        if (handle.kind === "file" && /\.(md|markdown)$/i.test(name)) {
          mdFiles.push({ name, handle });
        }
      }
      mdFiles.sort((a, b) => a.name.localeCompare(b.name));
      const preferred =
        mdFiles.find((f) => /^deck\.md$/i.test(f.name)) ||
        mdFiles.find((f) => /^readme\.md$/i.test(f.name)) ||
        mdFiles[0];
      if (preferred) {
        const file = await preferred.handle.getFile();
        const text = await file.text();
        state.fs.fileHandle = preferred.handle;
        state.fs.fileName = preferred.name;
        loadMarkdown(text, { clean: true });
      }
      // Load assets/ into virtual map
      const n = await loadAssetsFromFolder(dir);
      updateFileLabel();
      toast(
        preferred
          ? `Folder open · ${preferred.name}${n ? ` · ${n} assets` : ""}`
          : `Folder open · no .md found${n ? ` · ${n} assets` : ""}`
      );
    } catch (e) {
      if (e && e.name === "AbortError") return;
      toast("Folder open failed: " + e);
    }
  }

  async function loadAssetsFromFolder(dir) {
    let count = 0;
    try {
      let assetsDir = null;
      try {
        assetsDir = await dir.getDirectoryHandle("assets");
      } catch {
        /* no assets/ */
      }
      if (!assetsDir) return 0;
      for await (const [name, handle] of assetsDir.entries()) {
        if (handle.kind !== "file") continue;
        if (!/\.(png|jpe?g|gif|webp|svg)$/i.test(name)) continue;
        const file = await handle.getFile();
        const buf = await file.arrayBuffer();
        const b64 = arrayBufferToBase64(buf);
        const path = `assets/${name}`;
        state.assets[path] = b64;
        count++;
      }
      if (count) schedulePreview();
    } catch (e) {
      console.warn("load assets", e);
    }
    return count;
  }

  async function writeAssetsToFolder() {
    const dir = state.fs.dirHandle;
    if (!dir || !state.assets) return;
    const keys = Object.keys(state.assets);
    if (!keys.length) return;
    let assetsDir;
    try {
      assetsDir = await dir.getDirectoryHandle("assets", { create: true });
    } catch {
      return;
    }
    for (const path of keys) {
      const base = path.split("/").pop();
      if (!base) continue;
      const handle = await assetsDir.getFileHandle(base, { create: true });
      const writable = await handle.createWritable();
      const bytes = base64ToUint8(state.assets[path]);
      await writable.write(bytes);
      await writable.close();
    }
  }

  function arrayBufferToBase64(buf) {
    const bytes = new Uint8Array(buf);
    let bin = "";
    const chunk = 0x8000;
    for (let i = 0; i < bytes.length; i += chunk) {
      bin += String.fromCharCode.apply(null, bytes.subarray(i, i + chunk));
    }
    return btoa(bin);
  }

  function base64ToUint8(b64) {
    const bin = atob(b64);
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  }

  // ----- Brand kit from .potx / .pptx -----
  async function importBrandFromFile(file) {
    if (!file) return;
    setBadge("extracting brand…", "busy");
    try {
      const buf = await file.arrayBuffer();
      const b64 = arrayBufferToBase64(buf);
      const yaml = await engine.extractBrand({
        base64: b64,
        sourceName: file.name || "template.potx",
      });
      showBrandPanel(yaml, file.name);
      setBadge("live", "ok");
    } catch (e) {
      setBadge("brand failed", "err");
      toast("Brand extract failed: " + e);
    }
  }

  function parseOverlayFields(yaml) {
    const fields = {};
    for (const line of yaml.split("\n")) {
      const t = line.trim();
      if (!t || t.startsWith("#")) continue;
      const m = t.match(/^([A-Za-z0-9_]+)\s*:\s*(.+)$/);
      if (!m) continue;
      let v = m[2].trim();
      // strip trailing comment and quotes
      const hash = v.search(/\s+#/);
      if (hash >= 0) v = v.slice(0, hash).trim();
      if (
        (v.startsWith('"') && v.endsWith('"')) ||
        (v.startsWith("'") && v.endsWith("'"))
      ) {
        v = v.slice(1, -1);
      }
      fields[m[1]] = v;
    }
    return fields;
  }

  function showBrandPanel(yaml, sourceName) {
    const fields = parseOverlayFields(yaml);
    const wrap = document.createElement("div");
    wrap.className = "brand-panel";

    const intro = document.createElement("p");
    intro.className = "muted";
    intro.textContent = `Extracted from ${sourceName || "template"} — same as CLI theme extract. Apply as front-matter style: (every format).`;
    wrap.appendChild(intro);

    const swatches = document.createElement("div");
    swatches.className = "brand-swatches";
    for (const key of [
      "bg",
      "body_color",
      "title_color",
      "accent",
      "accent_soft",
      "divider",
      "link",
      "section_bg",
      "on_accent",
    ]) {
      if (!fields[key]) continue;
      const chip = document.createElement("div");
      chip.className = "brand-swatch";
      chip.title = `${key}: ${fields[key]}`;
      chip.innerHTML = `<span class="sw"></span><span class="sk"></span>`;
      chip.querySelector(".sw").style.background = fields[key];
      chip.querySelector(".sk").textContent = key;
      swatches.appendChild(chip);
    }
    wrap.appendChild(swatches);

    if (fields.title_font || fields.body_font) {
      const fonts = document.createElement("p");
      fonts.className = "muted";
      fonts.textContent = [
        fields.title_font && `title: ${fields.title_font}`,
        fields.body_font && `body: ${fields.body_font}`,
      ]
        .filter(Boolean)
        .join(" · ");
      wrap.appendChild(fonts);
    }

    const pre = document.createElement("pre");
    pre.className = "ir-pre brand-yaml";
    pre.textContent = yaml;
    wrap.appendChild(pre);

    const actions = document.createElement("div");
    actions.className = "brand-actions";
    const applyBtn = document.createElement("button");
    applyBtn.type = "button";
    applyBtn.className = "primary";
    applyBtn.textContent = "Apply to document";
    applyBtn.addEventListener("click", () => {
      applyBrandToDocument(yaml, fields);
      toast("Brand applied (style: in front-matter)");
      closeDrawer();
    });
    const dlBtn = document.createElement("button");
    dlBtn.type = "button";
    dlBtn.textContent = "Download brand.yaml";
    dlBtn.addEventListener("click", () => {
      const blob = new Blob([yaml], { type: "text/yaml;charset=utf-8" });
      const a = document.createElement("a");
      a.href = URL.createObjectURL(blob);
      a.download = "brand.yaml";
      document.body.appendChild(a);
      a.click();
      a.remove();
      setTimeout(() => URL.revokeObjectURL(a.href), 2000);
    });
    actions.appendChild(applyBtn);
    actions.appendChild(dlBtn);
    wrap.appendChild(actions);

    openDrawer("Brand kit", wrap);
  }

  function applyBrandToDocument(yaml, fields) {
    // Infer base theme from background luma.
    const bg = (fields.bg || "#FFFFFF").replace("#", "");
    let base = "light";
    if (/^[0-9a-fA-F]{6}$/.test(bg)) {
      const r = parseInt(bg.slice(0, 2), 16) / 255;
      const g = parseInt(bg.slice(2, 4), 16) / 255;
      const b = parseInt(bg.slice(4, 6), 16) / 255;
      const luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
      base = luma >= 0.5 ? "light" : "dark";
    }
    // Build style: block from non-comment overlay keys (skip pdf_* path keys).
    const styleLines = [];
    for (const line of yaml.split("\n")) {
      const t = line.trim();
      if (!t || t.startsWith("#")) continue;
      const m = t.match(/^([A-Za-z0-9_]+)\s*:\s*(.+)$/);
      if (!m) continue;
      if (m[1].startsWith("pdf_")) continue;
      let v = m[2].trim();
      const hash = v.search(/\s+#/);
      if (hash >= 0) v = v.slice(0, hash).trim();
      styleLines.push(`  ${m[1]}: ${v}`);
    }
    let md = $("editor").value;
    md = setFrontMatterField(md, "theme", base);
    md = upsertStyleBlock(md, styleLines.join("\n"));
    loadMarkdown(md);
    // Keep brand.yaml as a virtual asset for download/share.
    try {
      state.assets["brand.yaml"] = btoa(unescape(encodeURIComponent(yaml)));
    } catch {
      /* ignore */
    }
  }

  function upsertStyleBlock(md, styleBody) {
    const lines = md.split("\n");
    if (!lines.length || lines[0].trim() !== "---") {
      return `---\ntheme: light\nstyle:\n${styleBody}\n---\n\n${md}`;
    }
    let end = -1;
    for (let i = 1; i < lines.length; i++) {
      if (lines[i].trim() === "---") {
        end = i;
        break;
      }
    }
    if (end < 0) {
      return `---\nstyle:\n${styleBody}\n---\n${md}`;
    }
    // Remove existing style: block (style: + indented lines).
    const fm = lines.slice(1, end);
    const cleaned = [];
    let skipping = false;
    for (const line of fm) {
      if (/^style\s*:/.test(line)) {
        skipping = true;
        continue;
      }
      if (skipping) {
        if (/^\s+/.test(line) || line.trim() === "") continue;
        skipping = false;
      }
      cleaned.push(line);
    }
    cleaned.push("style:");
    cleaned.push(...styleBody.split("\n"));
    return [lines[0], ...cleaned, ...lines.slice(end)].join("\n");
  }

  // ----- Filmstrip scrub -----
  let filmstripSig = "";

  function renderFilmstrip(slides) {
    const track = $("#filmstrip-track");
    if (!track) return;
    const list = slides || state.slides || [];
    const sig =
      list.map((s) => `${s.index}:${s.title}:${s.kind}`).join("|") +
      `|a${state.activeSlide}`;
    // Rebuild structure only when outline changes; still refresh active class cheaply.
    const structSig = list.map((s) => `${s.index}:${s.title}:${s.kind}`).join("|");
    if (structSig !== filmstripSig) {
      filmstripSig = structSig;
      track.innerHTML = "";
      list.forEach((s, i) => {
        const card = document.createElement("button");
        card.type = "button";
        card.className = "film-card";
        card.dataset.index = String(s.index);
        card.innerHTML =
          `<span class="film-n"></span>` +
          `<span class="film-thumb" data-kind=""></span>` +
          `<span class="film-t"></span>`;
        card.querySelector(".film-n").textContent = String(s.index);
        card.querySelector(".film-t").textContent = s.title || `Slide ${s.index}`;
        card.querySelector(".film-thumb").dataset.kind = s.kind || "content";
        card.title = `${s.index}. ${s.title || ""} (${s.kind || "content"})`;
        card.addEventListener("click", () => {
          state.activeSlide = s.index;
          state.preview.focusIndex = Math.max(0, (s.index | 0) - 1);
          // Mirror rail click: flash source + re-window preview.
          const host = $("slide-list");
          const railBtn = host?.querySelectorAll(".slide-item")[i];
          if (railBtn) railBtn.click();
          else schedulePreview();
          highlightFilmstrip();
        });
        track.appendChild(card);
      });
    }
    highlightFilmstrip();
  }

  function highlightFilmstrip() {
    const track = $("#filmstrip-track");
    if (!track) return;
    const active = state.activeSlide | 0;
    let activeEl = null;
    track.querySelectorAll(".film-card").forEach((card) => {
      const on = (card.dataset.index | 0) === active;
      card.classList.toggle("active", on);
      if (on) activeEl = card;
    });
    if (activeEl) {
      activeEl.scrollIntoView({
        behavior: "smooth",
        block: "nearest",
        inline: "center",
      });
    }
  }

  // ----- Wire DOM -----
  function wire() {
    renderGallery();
    updateMathBadge();
    updateReviewChips();
    maybeShowEmpty();
    updateFileLabel();

    ui.mathBadge?.addEventListener("click", () => cycleMathMode());
    $("#btn-palette")?.addEventListener("click", () => openPalette());
    $("#btn-share")?.addEventListener("click", () => shareSnapshot());
    $("#btn-drawer-close")?.addEventListener("click", () => closeDrawer());
    $("#btn-talk-exit")?.addEventListener("click", () => toggleTalkMode(false));
    $("#btn-save")?.addEventListener("click", () => saveMarkdownFile());
    $("#btn-folder")?.addEventListener("click", () => openFolder());
    $("#btn-brand")?.addEventListener("click", () => $("#brand-input")?.click());
    $("#brand-input")?.addEventListener("change", async (e) => {
      const file = e.target.files?.[0];
      if (file) await importBrandFromFile(file);
      e.target.value = "";
    });
    // Prefer FSA open over plain file input
    $("#btn-open")?.addEventListener(
      "click",
      (e) => {
        e.stopImmediatePropagation();
        e.preventDefault();
        openMarkdownFile();
      },
      true
    );
    $("file-input")?.addEventListener("change", async (e) => {
      const file = e.target.files?.[0];
      if (!file) return;
      state.fs.fileHandle = null;
      state.fs.fileName = file.name;
      loadMarkdown(await file.text(), { clean: true });
      toast(`Opened ${file.name}`);
    });

    ui.paletteInput?.addEventListener("input", () => {
      paletteSel = 0;
      renderPaletteList();
    });
    ui.paletteInput?.addEventListener("keydown", (e) => {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        paletteMove(1);
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        paletteMove(-1);
      } else if (e.key === "Enter") {
        e.preventDefault();
        paletteConfirm();
      } else if (e.key === "Escape") {
        e.preventDefault();
        closePalette();
      }
    });

    // Export recipes in menu
    document.querySelectorAll("[data-recipe]").forEach((btn) => {
      btn.addEventListener("click", (e) => {
        e.stopPropagation();
        const r = btn.getAttribute("data-recipe");
        if (r === "board") recipeBoardPack();
        if (r === "all") recipeAllFormats();
      });
    });

    // Drop images on editor
    const shell = document.querySelector(".editor-shell");
    shell?.addEventListener("dragover", (e) => {
      e.preventDefault();
      shell.classList.add("drop-target");
    });
    shell?.addEventListener("dragleave", () => shell.classList.remove("drop-target"));
    shell?.addEventListener("drop", async (e) => {
      e.preventDefault();
      shell.classList.remove("drop-target");
      if (e.dataTransfer?.files?.length) await handleImageFiles(e.dataTransfer.files);
    });

    $("editor")?.addEventListener("paste", async (e) => {
      const items = e.clipboardData?.items;
      if (!items) return;
      const images = [];
      for (const it of items) {
        if (it.type.startsWith("image/")) {
          const f = it.getAsFile();
          if (f) images.push(f);
        }
      }
      if (images.length) {
        e.preventDefault();
        await handleImageFiles(images);
        return;
      }
      const text = e.clipboardData.getData("text/plain");
      if (text && text.includes("\t") && text.includes("\n")) {
        const table = tsvToMarkdownTable(text);
        if (table) {
          e.preventDefault();
          insertAtCaret(table + "\n");
          toast("Pasted as markdown table");
        }
      }
    });

    // Image file input
    $("#btn-image")?.addEventListener("click", () => $("#image-input")?.click());
    $("#image-input")?.addEventListener("change", async (e) => {
      const files = e.target.files;
      if (files?.length) await handleImageFiles(files);
      e.target.value = "";
    });

    document.addEventListener("keydown", (e) => {
      const tag = (e.target && e.target.tagName) || "";
      const inField =
        tag === "TEXTAREA" || tag === "INPUT" || e.target?.isContentEditable;

      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        openPalette();
        return;
      }
      // ⌘S → save markdown; ⌘⇧S → export PDF; ⌘⇧E → PPTX
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "s") {
        e.preventDefault();
        e.stopImmediatePropagation();
        if (e.shiftKey) exportFormat("pdf");
        else saveMarkdownFile();
        return;
      }
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key.toLowerCase() === "e") {
        e.preventDefault();
        exportFormat("pptx");
        return;
      }
      if (e.key === "Escape") {
        closePalette();
        closeDrawer();
        toggleTalkMode(false);
        return;
      }
      if (!inField && (e.key === "t" || e.key === "T")) {
        e.preventDefault();
        toggleTalkMode(true);
      }
      if (!inField && e.key === "?") {
        e.preventDefault();
        openPalette("help");
      }
    });

    // Observe editor changes for badge / review / empty / dirty
    $("editor")?.addEventListener("input", () => {
      updateMathBadge();
      updateReviewChips();
      maybeShowEmpty();
      markDirty();
    });

    // Hook rail after each render — app calls extras.onRailRendered
  }

  // Export last-export hash stamp ("diff since last export")
  let lastExportHash = null;

  function simpleHash(s) {
    let h = 2166136261;
    for (let i = 0; i < s.length; i++) {
      h ^= s.charCodeAt(i);
      h = Math.imul(h, 16777619);
    }
    return (h >>> 0).toString(16);
  }

  function dirtySinceExport() {
    if (!lastExportHash) return false;
    return simpleHash($("editor").value) !== lastExportHash;
  }

  function markExported() {
    lastExportHash = simpleHash($("editor").value);
    const el = $("#export-stamp");
    if (el) el.textContent = "in sync with export";
  }

  function refreshExportStamp() {
    const el = $("#export-stamp");
    if (!el) return;
    el.textContent = dirtySinceExport()
      ? "changed since export"
      : lastExportHash
        ? "in sync with export"
        : "not exported yet";
  }

  // Public hooks for app.js
  return {
    wire,
    tryLoadShareHash,
    onRailRendered: (host) => {
      enhanceRailList(host);
      renderFilmstrip(state.slides);
    },
    onPreviewApplied: () => {
      updateMathBadge();
      refreshExportStamp();
      renderFilmstrip(state.slides);
      markDirty();
    },
    markExported,
    markCleanBaseline,
    loadMarkdown,
    openPalette,
    openMarkdownFile,
    saveMarkdownFile,
    openFolder,
    exportFormat,
    TEMPLATES,
  };
}
