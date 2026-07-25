/**
 * Studio pro features: export ghost (HTML + SVG layout), speaker notes,
 * session fonts (OPFS), BYO AI, git helper.
 * Installed after studio-extras via installStudioPro(api).
 */

const AI_STORAGE = "md2any-studio:ai:v1";
const FONT_DIR = "md2any-fonts";
const GHOST_MODE_KEY = "md2any-studio:ghost-mode:v1";

const EDITOR_SYSTEM = `You are the writing assistant in md2any Studio (browser WASM).
The user edits a markdown slide deck that exports to PPTX/PDF/DOCX/etc.

Markup: optional YAML front-matter (theme, aspect, layout, math, style:).
# = section divider, ## = content slide, --- also splits slides.
Speaker notes: <!-- notes: ... -->. Math: $...$ and $$...$$.
Layout hints: <!-- layout: image-left|image-right --> etc.

EDIT PROTOCOL — surgical edits only:
When changing the deck, reply with a short summary then one or more blocks:
\`\`\`\`md2any op=replace slide=N
## Full slide markdown for slide N
\`\`\`\`
ops: replace | insert-after | insert-before | delete | replace-all
Never invent image URLs. Prefer keeping existing content verbatim when tweaking.
If only answering a question, plain prose — no op blocks.`;

function loadAiSettings() {
  try {
    return JSON.parse(localStorage.getItem(AI_STORAGE) || "{}") || {};
  } catch {
    return {};
  }
}

function saveAiSettings(s) {
  try {
    localStorage.setItem(AI_STORAGE, JSON.stringify(s));
  } catch {
    /* quota */
  }
}

export function installStudioPro(api) {
  const {
    $,
    state,
    engine,
    schedulePreview,
    exportFormat,
    setBadge,
    parseFrontMatter,
  } = api;

  function toastMsg(msg, ms = 2200) {
    const el = $("#toast");
    if (!el) return;
    el.textContent = msg;
    el.classList.add("show");
    clearTimeout(el._t);
    el._t = setTimeout(() => el.classList.remove("show"), ms);
  }

  // -------------------------------------------------------------------------
  // Export ghost — HTML full export OR SVG layout (CLI / PPTX-like geometry)
  // -------------------------------------------------------------------------
  let ghostMode = "html";
  try {
    const saved = localStorage.getItem(GHOST_MODE_KEY);
    if (saved === "svg" || saved === "html") ghostMode = saved;
  } catch {
    /* ignore */
  }

  state.ghost = state.ghost || {
    enabled: false,
    timer: null,
    gen: 0,
    stamp: null,
    busy: false,
    mode: ghostMode,
  };
  state.ghost.mode = ghostMode;
  /** @type {Map<number, string>} index → data URL for filmstrip thumbs */
  state.thumbCache = state.thumbCache || new Map();

  function setGhostEnabled(on) {
    state.ghost.enabled = !!on;
    document.body.classList.toggle("ghost-on", state.ghost.enabled);
    const btn = $("#btn-ghost");
    if (btn) btn.classList.toggle("active", state.ghost.enabled);
    const pane = $("#ghost-pane");
    if (pane) pane.hidden = !state.ghost.enabled;
    syncGhostModeButtons();
    if (state.ghost.enabled) {
      scheduleGhost(200);
      toastMsg(
        state.ghost.mode === "svg"
          ? "Export ghost on — SVG layout (PPTX-like)"
          : "Export ghost on — HTML export"
      );
    } else {
      toastMsg("Export ghost off");
    }
    updateGhostMeta();
  }

  function toggleGhost() {
    setGhostEnabled(!state.ghost.enabled);
  }

  function setGhostMode(mode) {
    state.ghost.mode = mode === "svg" ? "svg" : "html";
    try {
      localStorage.setItem(GHOST_MODE_KEY, state.ghost.mode);
    } catch {
      /* ignore */
    }
    syncGhostModeButtons();
    if (state.ghost.enabled) scheduleGhost(100);
    updateGhostMeta();
  }

  function syncGhostModeButtons() {
    document.querySelectorAll("[data-ghost-mode]").forEach((btn) => {
      btn.classList.toggle(
        "active",
        btn.getAttribute("data-ghost-mode") === state.ghost.mode
      );
    });
  }

  function scheduleGhost(ms = 1200) {
    if (!state.ghost.enabled || !state.ready) return;
    clearTimeout(state.ghost.timer);
    state.ghost.timer = setTimeout(() => runGhost(), ms);
  }

  function updateGhostMeta() {
    const el = $("#ghost-meta");
    if (!el) return;
    if (!state.ghost.enabled) {
      el.textContent = "";
      return;
    }
    if (state.ghost.busy) {
      el.textContent = `rendering ${state.ghost.mode}…`;
      return;
    }
    el.textContent = state.ghost.stamp
      ? `${state.ghost.mode} · ${state.ghost.stamp}`
      : `${state.ghost.mode} · waiting`;
  }

  function assetsJson() {
    return state.assets && Object.keys(state.assets).length
      ? JSON.stringify(state.assets)
      : null;
  }

  function b64ToBytes(b64) {
    const bin = atob(b64);
    const bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
    return bytes;
  }

  async function runGhost() {
    if (!state.ghost.enabled || state.ghost.busy || !state.ready) return;
    state.ghost.busy = true;
    state.ghost.gen++;
    const gen = state.ghost.gen;
    updateGhostMeta();
    try {
      if (state.ghost.mode === "svg") {
        await runSvgGhost(gen);
      } else {
        await runHtmlGhost(gen);
      }
      if (gen !== state.ghost.gen) return;
      state.ghost.stamp = new Date().toLocaleTimeString();
    } catch (e) {
      console.warn("ghost", e);
      const meta = $("#ghost-meta");
      if (meta) meta.textContent = "ghost failed: " + (e.message || e);
    } finally {
      state.ghost.busy = false;
      updateGhostMeta();
    }
  }

  async function runHtmlGhost(gen) {
    const result = await engine.convert({
      markdown: $("editor").value,
      format: "html",
      theme: state.theme,
      aspect: state.aspect,
      layout: state.layout,
      assetsJson: assetsJson(),
    });
    if (gen !== state.ghost.gen) return;
    const html = new TextDecoder().decode(b64ToBytes(result.base64));
    const iframe = $("#ghost-frame");
    if (iframe) iframe.srcdoc = html;
    iframe?.addEventListener(
      "load",
      () => {
        try {
          const doc = iframe.contentDocument;
          const slides = doc?.querySelectorAll?.(".slide");
          const idx = Math.max(0, (state.activeSlide | 0) - 1);
          slides?.[idx]?.scrollIntoView?.({ block: "start" });
        } catch {
          /* sandbox */
        }
      },
      { once: true }
    );
  }

  async function runSvgGhost(gen) {
    // Window around active slide for speed; full deck would freeze huge talks.
    const n = Math.max(1, (state.slides || []).length);
    const active0 = Math.max(0, (state.activeSlide | 0) - 1);
    const radius = 2;
    const from = Math.max(0, active0 - radius);
    const to = Math.min(n, active0 + radius + 1);
    const imgs = await engine.slideImages({
      markdown: $("editor").value,
      theme: state.theme,
      aspect: state.aspect,
      layout: state.layout,
      assetsJson: assetsJson(),
      from,
      to,
      format: "svg",
    });
    if (gen !== state.ghost.gen) return;
    const list = Array.isArray(imgs) ? imgs : [];
    // Warm filmstrip thumbs for rendered slides.
    for (const s of list) {
      if (!s?.base64) continue;
      const url = `data:image/svg+xml;base64,${s.base64}`;
      state.thumbCache.set(s.index | 0, url);
    }
    paintFilmstripThumbs();
    const cards = list
      .map((s) => {
        const url = `data:image/svg+xml;base64,${s.base64}`;
        const active = (s.index | 0) === (state.activeSlide | 0);
        return `<figure class="ghost-slide${active ? " active" : ""}" data-i="${s.index}">
          <figcaption>${s.index}. ${escapeHtml(s.title || "")}</figcaption>
          <img src="${url}" alt="Slide ${s.index}" loading="lazy"/>
        </figure>`;
      })
      .join("\n");
    const html = `<!doctype html><html><head><meta charset="utf-8"/>
<style>
  body{margin:0;background:#0b0d12;color:#c8cdd8;font:12px/1.3 system-ui,sans-serif}
  .wrap{display:flex;flex-direction:column;gap:1rem;padding:1rem;align-items:center}
  .ghost-slide{margin:0;width:min(100%,920px);border:1px solid #2a3140;border-radius:8px;overflow:hidden;background:#111}
  .ghost-slide.active{border-color:#6ea8fe;box-shadow:0 0 0 1px #6ea8fe55}
  .ghost-slide img{display:block;width:100%;height:auto;background:#fff}
  figcaption{padding:.4rem .6rem;background:#151a24;color:#9aa3b5}
  .hint{opacity:.7;padding:0 1rem}
</style></head><body>
<p class="hint">SVG export geometry (same IR as CLI --format svg) · slides ${from + 1}–${to} of ${n}</p>
<div class="wrap">${cards || "<p>No slides</p>"}</div>
</body></html>`;
    const iframe = $("#ghost-frame");
    if (iframe) iframe.srcdoc = html;
    iframe?.addEventListener(
      "load",
      () => {
        try {
          const doc = iframe.contentDocument;
          doc?.querySelector?.(".ghost-slide.active")?.scrollIntoView?.({
            block: "center",
          });
        } catch {
          /* sandbox */
        }
      },
      { once: true }
    );
  }

  function paintFilmstripThumbs() {
    const track = $("#filmstrip-track");
    if (!track || !state.thumbCache?.size) return;
    track.querySelectorAll(".film-card").forEach((card) => {
      const idx = card.dataset.index | 0;
      const url = state.thumbCache.get(idx);
      const thumb = card.querySelector(".film-thumb");
      if (!thumb || !url) return;
      thumb.style.backgroundImage = `url("${url}")`;
      thumb.style.backgroundSize = "cover";
      thumb.style.backgroundPosition = "center";
      thumb.classList.add("has-thumb");
    });
  }

  /** Idle job: fill filmstrip thumbs via SVG for visible deck (capped). */
  let thumbTimer = null;
  function scheduleFilmstripThumbs(ms = 1800) {
    if (!state.ready) return;
    clearTimeout(thumbTimer);
    thumbTimer = setTimeout(() => warmFilmstripThumbs(), ms);
  }

  async function warmFilmstripThumbs() {
    const n = (state.slides || []).length;
    if (!n || n > 80) return; // skip huge decks in background
    try {
      const imgs = await engine.slideImages({
        markdown: $("editor").value,
        theme: state.theme,
        aspect: state.aspect,
        layout: state.layout,
        assetsJson: assetsJson(),
        from: 0,
        to: n,
        format: "svg",
      });
      for (const s of imgs || []) {
        if (!s?.base64) continue;
        state.thumbCache.set(s.index | 0, `data:image/svg+xml;base64,${s.base64}`);
      }
      paintFilmstripThumbs();
    } catch (e) {
      console.warn("filmstrip thumbs", e);
    }
  }

  // -------------------------------------------------------------------------
  // BYO AI (browser key → OpenAI-compatible endpoint)
  // -------------------------------------------------------------------------
  let chatHistory = [];
  let chatBusy = false;

  function openAiSettings() {
    const s = loadAiSettings();
    const wrap = document.createElement("div");
    wrap.className = "ai-settings";
    wrap.innerHTML = `
      <p class="muted">Key stays in <strong>localStorage</strong> on this device.
      The model is called from your browser (or via local <code>md2any --studio</code> proxy if the host supports it).
      Providers without CORS need the CLI proxy.</p>
      <label>API key<br/><input id="ai-key" type="password" autocomplete="off" spellcheck="false" /></label>
      <label>Endpoint<br/><input id="ai-endpoint" type="url" /></label>
      <label>Model<br/><input id="ai-model" type="text" /></label>
      <div class="brand-actions" style="margin-top:0.75rem">
        <button type="button" class="primary" id="ai-save-settings">Save</button>
        <button type="button" id="ai-clear-settings">Clear</button>
      </div>
    `;
    wrap.querySelector("#ai-key").value = s.apiKey || "";
    wrap.querySelector("#ai-endpoint").value =
      s.endpoint || "https://api.openai.com/v1/chat/completions";
    wrap.querySelector("#ai-model").value = s.model || "gpt-4o-mini";
    wrap.querySelector("#ai-save-settings").addEventListener("click", () => {
      saveAiSettings({
        apiKey: wrap.querySelector("#ai-key").value.trim(),
        endpoint: wrap.querySelector("#ai-endpoint").value.trim(),
        model: wrap.querySelector("#ai-model").value.trim(),
      });
      toastMsg("AI settings saved locally");
      closeDrawerSafe();
    });
    wrap.querySelector("#ai-clear-settings").addEventListener("click", () => {
      saveAiSettings({});
      wrap.querySelector("#ai-key").value = "";
      toastMsg("AI settings cleared");
    });
    openDrawerSafe("AI settings (BYO key)", wrap);
  }

  function openAiChat() {
    const s = loadAiSettings();
    if (!s.apiKey) {
      openAiSettings();
      toastMsg("Add an API key first");
      return;
    }
    const wrap = document.createElement("div");
    wrap.className = "ai-chat";
    wrap.innerHTML = `
      <div class="ai-chip" id="ai-active-chip">Target: active slide (or whole deck)</div>
      <div class="ai-log" id="ai-log"></div>
      <div class="ai-row">
        <textarea id="ai-input" rows="2" placeholder="Rewrite this slide to be shorter…"></textarea>
        <button type="button" class="primary" id="ai-send">Send</button>
      </div>
      <p class="muted">Surgical ops: model returns <code>md2any op=replace slide=N</code> blocks. Apply with one click.</p>
    `;
    openDrawerSafe("AI assistant", wrap);
    const log = wrap.querySelector("#ai-log");
    const input = wrap.querySelector("#ai-input");
    const send = wrap.querySelector("#ai-send");
    const chip = wrap.querySelector("#ai-active-chip");
    const active = state.activeSlide || 1;
    const title = state.slides?.[active - 1]?.title || "";
    chip.textContent = `Target: slide ${active}${title ? " · " + title : ""}`;

    function addMsg(cls, text) {
      const d = document.createElement("div");
      d.className = "ai-msg " + cls;
      d.textContent = text;
      log.appendChild(d);
      log.scrollTop = log.scrollHeight;
      return d;
    }

    send.addEventListener("click", () => doSend());
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        doSend();
      }
    });

    async function doSend() {
      const text = input.value.trim();
      if (!text || chatBusy) return;
      input.value = "";
      addMsg("user", text);
      chatHistory.push({ role: "user", content: text });
      chatBusy = true;
      send.disabled = true;
      const bubble = addMsg("bot busy", "thinking…");
      try {
        const reply = await callAi(chatHistory, {
          doc: $("editor").value,
          slides: (state.slides || []).map((s) => ({
            n: s.index,
            title: s.title,
          })),
          active: state.activeSlide || null,
        });
        bubble.classList.remove("busy");
        chatHistory.push({ role: "assistant", content: reply });
        finalizeBot(bubble, reply);
      } catch (e) {
        bubble.remove();
        addMsg("err", String(e.message || e));
      } finally {
        chatBusy = false;
        send.disabled = false;
      }
    }
  }

  async function callAi(history, ctx) {
    const s = loadAiSettings();
    if (!s.apiKey) throw new Error("No API key — open AI settings");
    const slides = (ctx.slides || [])
      .map((x) => `${x.n}. ${x.title}`)
      .join("\n");
    const userBlob = [
      "CURRENT DOCUMENT:",
      "-----",
      ctx.doc,
      "-----",
      "SLIDE LIST:",
      slides || "(empty)",
      ctx.active != null ? `SELECTED SLIDE: ${ctx.active}` : "SELECTED SLIDE: none",
      "",
      "User message:",
      history[history.length - 1]?.content || "",
    ].join("\n");

    const messages = [
      { role: "system", content: EDITOR_SYSTEM },
      // Prior turns (skip last user — already in blob)
      ...history.slice(0, -1).map((m) => ({ role: m.role, content: m.content })),
      { role: "user", content: userBlob },
    ];

    // Prefer local studio proxy (avoids CORS); fall back to direct fetch.
    const proxyBody = {
      endpoint: s.endpoint,
      model: s.model,
      apiKey: s.apiKey,
      messages,
    };
    try {
      const pr = await fetch("./__studio_chat", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(proxyBody),
      });
      if (pr.ok) {
        const data = await pr.json();
        if (data.error) throw new Error(data.error);
        if (data.content) return data.content;
      }
      // 404 = plain static host — fall through
    } catch (e) {
      if (e && e.message && !String(e.message).includes("Failed to fetch")) {
        // proxy returned error JSON path already thrown
      }
    }

    const r = await fetch(s.endpoint, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Authorization: `Bearer ${s.apiKey}`,
      },
      body: JSON.stringify({
        model: s.model,
        temperature: 0.5,
        messages,
      }),
    });
    const text = await r.text();
    if (!r.ok) {
      let detail = text;
      try {
        const j = JSON.parse(text);
        detail = j.error?.message || text;
      } catch {
        /* raw */
      }
      throw new Error(`API HTTP ${r.status}: ${detail}`);
    }
    const j = JSON.parse(text);
    const content = j.choices?.[0]?.message?.content;
    if (!content) throw new Error("Empty model response");
    return content;
  }

  function extractOps(reply) {
    const ops = [];
    const re =
      /(`{3,4})[ \t]*md2any[ \t]+op=([a-z-]+)(?:[ \t]+slide=(\d+))?[^\n]*\r?\n([\s\S]*?)\1/g;
    let m;
    while ((m = re.exec(reply)) !== null) {
      ops.push({
        op: m[2],
        n: m[3] ? Number(m[3]) : null,
        content: m[4].replace(/\s+$/, ""),
      });
    }
    return ops;
  }

  function parseDeckBlocks(md) {
    const lines = md.split("\n");
    let fmEnd = 0;
    if (lines[0]?.trim() === "---") {
      for (let i = 1; i < lines.length; i++) {
        if (lines[i].trim() === "---") {
          fmEnd = i + 1;
          break;
        }
      }
    }
    const isStart = (l) => /^#{1,2}\s/.test(l) || /^---\s*$/.test(l);
    const starts = [];
    let inCode = false;
    for (let k = fmEnd; k < lines.length; k++) {
      if (/^\s*(```|~~~)/.test(lines[k])) {
        inCode = !inCode;
        continue;
      }
      if (!inCode && isStart(lines[k])) starts.push(k);
    }
    const blocks = [];
    const titleEnd = starts.length ? starts[0] : lines.length;
    blocks.push({ start: 0, end: titleEnd, title: "Title" });
    for (let s = 0; s < starts.length; s++) {
      const start = starts[s];
      const end = s + 1 < starts.length ? starts[s + 1] : lines.length;
      let t = /^---\s*$/.test(lines[start])
        ? "(rule)"
        : lines[start].replace(/^#{1,2}\s*/, "").trim();
      blocks.push({ start, end, title: t || "(untitled)" });
    }
    return { lines, blocks };
  }

  function applyOps(ops) {
    const ta = $("editor");
    const all = ops.find((o) => o.op === "replace-all");
    if (all) {
      ta.value = all.content;
      schedulePreview();
      return true;
    }
    const { lines, blocks } = parseDeckBlocks(ta.value);
    const actions = [];
    for (const o of ops) {
      if (o.n == null) continue;
      const b = blocks[o.n - 1];
      if (!b) continue;
      if (o.op === "replace")
        actions.push({ start: b.start, end: b.end, text: o.content });
      else if (o.op === "delete")
        actions.push({ start: b.start, end: b.end, text: null });
      else if (o.op === "insert-after")
        actions.push({ start: b.end, end: b.end, text: o.content });
      else if (o.op === "insert-before")
        actions.push({ start: b.start, end: b.start, text: o.content });
    }
    actions.sort((a, b) => b.start - a.start);
    for (const a of actions) {
      const repl = a.text == null ? [] : a.text.split("\n").concat([""]);
      lines.splice(a.start, a.end - a.start, ...repl);
    }
    ta.value = lines.join("\n").replace(/\n{3,}/g, "\n\n");
    schedulePreview();
    return true;
  }

  function finalizeBot(bubble, reply) {
    const ops = extractOps(reply);
    if (!ops.length) {
      bubble.textContent = reply;
      return;
    }
    const i = reply.search(/`{3,4}[ \t]*md2any/);
    const summary =
      (i > 0 ? reply.slice(0, i).trim() : "") ||
      `Proposed ${ops.length} edit${ops.length > 1 ? "s" : ""}.`;
    bubble.textContent = summary;
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "ai-apply";
    btn.textContent =
      ops.length === 1 && ops[0].op === "replace-all"
        ? "✓ Apply"
        : `✓ Apply ${ops.length} edit${ops.length > 1 ? "s" : ""}`;
    btn.addEventListener("click", () => {
      applyOps(ops);
      btn.textContent = "✓ Applied";
      btn.disabled = true;
      toastMsg("AI edits applied");
    });
    bubble.appendChild(document.createElement("br"));
    bubble.appendChild(btn);
  }

  // -------------------------------------------------------------------------
  // Git helper
  // -------------------------------------------------------------------------
  async function openGitHelper() {
    const md = $("editor").value;
    const { fields } = parseFrontMatter(md);
    const title = fields.title || state.slides?.[0]?.title || "deck";
    const n = state.slides?.length || 0;
    const name = state.fs?.fileName || "deck.md";
    const dirty = state.fs?.dirty ? "modified" : "clean";
    const msg = `docs: update ${name} — ${title} (${n} slides)`;

    let remoteStatus = null;
    try {
      const r = await fetch("./__studio_git_status", { cache: "no-store" });
      if (r.ok) remoteStatus = await r.json();
    } catch {
      /* static host */
    }

    const wrap = document.createElement("div");
    wrap.className = "git-panel";
    const statusLines = remoteStatus
      ? `<pre class="ir-pre">${escapeHtml(
          remoteStatus.summary || JSON.stringify(remoteStatus, null, 2)
        )}</pre>`
      : `<p class="muted">No git status from host. Use with <code>md2any path.md --studio</code> for live status, or copy the command below into your terminal.</p>`;

    wrap.innerHTML = `
      <p class="muted">File: <strong>${escapeHtml(name)}</strong> · ${dirty} · ${n} slides</p>
      ${statusLines}
      <label>Commit message<br/>
        <input id="git-msg" type="text" />
      </label>
      <pre class="ir-pre" id="git-cmd"></pre>
      <div class="brand-actions">
        <button type="button" class="primary" id="git-copy">Copy command</button>
        <button type="button" id="git-copy-msg">Copy message only</button>
        ${
          remoteStatus?.can_commit
            ? `<button type="button" id="git-commit">Commit via CLI host</button>`
            : ""
        }
      </div>
    `;
    wrap.querySelector("#git-msg").value = msg;
    const cmdEl = wrap.querySelector("#git-cmd");
    const refreshCmd = () => {
      const m = wrap.querySelector("#git-msg").value.replace(/"/g, '\\"');
      cmdEl.textContent = `git add ${name} && git commit -m "${m}"`;
    };
    refreshCmd();
    wrap.querySelector("#git-msg").addEventListener("input", refreshCmd);
    wrap.querySelector("#git-copy").addEventListener("click", async () => {
      await navigator.clipboard.writeText(cmdEl.textContent);
      toastMsg("Commit command copied");
    });
    wrap.querySelector("#git-copy-msg").addEventListener("click", async () => {
      await navigator.clipboard.writeText(wrap.querySelector("#git-msg").value);
      toastMsg("Message copied");
    });
    const commitBtn = wrap.querySelector("#git-commit");
    if (commitBtn) {
      commitBtn.addEventListener("click", async () => {
        if (!confirm("Create a git commit via the local studio host?")) return;
        try {
          // Save markdown first if FSA
          if (window.__studioExtras?.saveMarkdownFile) {
            await window.__studioExtras.saveMarkdownFile();
          }
          const r = await fetch("./__studio_git_commit", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({
              message: wrap.querySelector("#git-msg").value,
            }),
          });
          const data = await r.json();
          if (!r.ok || data.error) throw new Error(data.error || r.statusText);
          toastMsg(data.ok || "Committed");
          closeDrawerSafe();
        } catch (e) {
          toastMsg("Commit failed: " + e);
        }
      });
    }
    openDrawerSafe("Git helper", wrap);
  }

  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function openDrawerSafe(title, node) {
    const drawer = $("#drawer");
    const body = $("#drawer-body");
    const h = $("#drawer-title");
    if (!drawer || !body) return;
    drawer.hidden = false;
    if (h) h.textContent = title;
    body.innerHTML = "";
    body.appendChild(node);
  }

  function closeDrawerSafe() {
    const drawer = $("#drawer");
    if (drawer) drawer.hidden = true;
  }

  // -------------------------------------------------------------------------
  // Speaker notes — panel + draft from IR / markdown body
  // -------------------------------------------------------------------------
  function fmEndLine(lines) {
    if (!lines.length || lines[0].trim() !== "---") return 0;
    for (let i = 1; i < lines.length; i++) {
      if (lines[i].trim() === "---") return i + 1;
    }
    return 0;
  }

  function slideBodyMarkdown(md, slideIndex1) {
    const { lines, blocks } = parseDeckBlocks(md);
    const i = (slideIndex1 | 0) - 1;
    if (i < 0 || i >= blocks.length) return "";
    const b = blocks[i];
    // Skip YAML front-matter if it lives inside the first block.
    let start = b.start;
    if (i === 0) start = Math.max(start, fmEndLine(lines));
    return lines.slice(start, b.end).join("\n");
  }

  function extractNotesFromSlideMd(slideMd) {
    const m = slideMd.match(/<!--\s*(?:speaker\s+)?notes:\s*([\s\S]*?)-->/i);
    return m ? m[1].trim() : "";
  }

  function draftNotesFromSlideMd(slideMd) {
    // Strip existing notes comments and fenced code for a talk track draft.
    const body = slideMd
      .replace(/<!--\s*(?:speaker\s+)?notes:[\s\S]*?-->/gi, "")
      .replace(/```[\s\S]*?```/g, "")
      .replace(/\$\$[\s\S]*?\$\$/g, "[equation]")
      .replace(/!\[[^\]]*\]\([^)]*\)/g, "")
      .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1");
    const lines = body.split("\n");
    const title =
      lines
        .map((l) => l.match(/^#{1,2}\s+(.+)/))
        .find(Boolean)?.[1]
        ?.trim() || "this slide";
    const bullets = [];
    for (const line of lines) {
      const b = line.match(/^\s*[-*+]\s+(.+)/) || line.match(/^\s*\d+\.\s+(.+)/);
      if (b) bullets.push(b[1].replace(/[*_`]/g, "").trim());
    }
    const paras = lines
      .map((l) => l.trim())
      .filter((l) => l && !/^#{1,6}\s/.test(l) && !/^[-*+|`]/.test(l) && !/^---+$/.test(l));
    const bits = [];
    bits.push(`Talk track for “${title}”.`);
    if (bullets.length) {
      bits.push("Hit these points:");
      bullets.slice(0, 8).forEach((b) => bits.push(`• ${b}`));
    } else if (paras.length) {
      bits.push(paras.slice(0, 3).join(" "));
    } else {
      bits.push("Add key talking points before you present.");
    }
    return bits.join("\n");
  }

  function setNotesOnSlide(md, slideIndex1, notesText) {
    const { lines, blocks } = parseDeckBlocks(md);
    const i = (slideIndex1 | 0) - 1;
    if (i < 0 || i >= blocks.length) return md;
    const b = blocks[i];
    // Keep front-matter intact when editing slide 1.
    const bodyStart = i === 0 ? Math.max(b.start, fmEndLine(lines)) : b.start;
    let slide = lines.slice(bodyStart, b.end).join("\n");
    const noteBlock = `<!-- notes:\n${notesText.trim()}\n-->`;
    if (!notesText.trim()) {
      slide = slide.replace(/\n?<!--\s*(?:speaker\s+)?notes:[\s\S]*?-->\n?/gi, "\n");
    } else if (/<!--\s*(?:speaker\s+)?notes:[\s\S]*?-->/i.test(slide)) {
      slide = slide.replace(/<!--\s*(?:speaker\s+)?notes:[\s\S]*?-->/i, noteBlock);
    } else {
      slide = slide.replace(/\s*$/, "\n\n" + noteBlock + "\n");
    }
    const repl = slide.split("\n");
    lines.splice(bodyStart, b.end - bodyStart, ...repl);
    return lines.join("\n").replace(/\n{3,}/g, "\n\n");
  }

  function openNotesPanel() {
    const wrap = document.createElement("div");
    wrap.className = "notes-panel";
    const active = state.activeSlide || 1;
    const slide = (state.slides || []).find((s) => s.index === active);
    const fromIr = (slide?.notes || "").trim();
    const fromMd = extractNotesFromSlideMd(slideBodyMarkdown($("editor").value, active));
    const current = fromIr || fromMd;
    wrap.innerHTML = `
      <p class="muted">Speaker notes use <code>&lt;!-- notes: … --&gt;</code> on each slide.
      They export to PPTX/ODP. Draft from IR pulls bullets from the active slide.</p>
      <div class="ai-chip">Slide ${active}${slide?.title ? " · " + escapeHtml(slide.title) : ""}${
        slide?.hasNotes || current ? " · has notes" : " · no notes"
      }</div>
      <label>Notes<br/><textarea id="notes-body" rows="10" spellcheck="true"></textarea></label>
      <div class="brand-actions" style="margin-top:0.75rem;flex-wrap:wrap;gap:0.4rem">
        <button type="button" class="primary" id="notes-apply">Apply to slide</button>
        <button type="button" id="notes-draft">Draft from IR</button>
        <button type="button" id="notes-ai">AI draft (BYO key)</button>
        <button type="button" id="notes-clear">Clear</button>
      </div>
    `;
    wrap.querySelector("#notes-body").value = current;
    wrap.querySelector("#notes-apply").addEventListener("click", () => {
      const text = wrap.querySelector("#notes-body").value;
      $("editor").value = setNotesOnSlide($("editor").value, active, text);
      schedulePreview();
      toastMsg("Notes applied to slide " + active);
      openNotesPanel();
    });
    wrap.querySelector("#notes-draft").addEventListener("click", () => {
      const body = slideBodyMarkdown($("editor").value, active);
      wrap.querySelector("#notes-body").value = draftNotesFromSlideMd(body);
      toastMsg("Drafted from slide content");
    });
    wrap.querySelector("#notes-clear").addEventListener("click", () => {
      wrap.querySelector("#notes-body").value = "";
    });
    wrap.querySelector("#notes-ai").addEventListener("click", async () => {
      const s = loadAiSettings();
      if (!s.apiKey) {
        openAiSettings();
        toastMsg("Add an API key first");
        return;
      }
      const body = slideBodyMarkdown($("editor").value, active);
      const btn = wrap.querySelector("#notes-ai");
      btn.disabled = true;
      btn.textContent = "Drafting…";
      try {
        const reply = await callAi(
          [
            {
              role: "user",
              content:
                "Write concise speaker notes (bullet talking points, 4–8 lines) for the SELECTED slide only. " +
                "Reply with plain text notes only — no markdown fences, no op blocks.",
            },
          ],
          {
            doc: body,
            slides: (state.slides || []).map((x) => ({ n: x.index, title: x.title })),
            active,
          }
        );
        wrap.querySelector("#notes-body").value = reply.trim();
        toastMsg("AI draft ready — Apply to save");
      } catch (e) {
        toastMsg("AI notes failed: " + (e.message || e));
      } finally {
        btn.disabled = false;
        btn.textContent = "AI draft (BYO key)";
      }
    });
    openDrawerSafe("Speaker notes", wrap);
  }

  function draftNotesAllFromIr() {
    const md = $("editor").value;
    const { lines, blocks } = parseDeckBlocks(md);
    const fm = fmEndLine(lines);
    // Apply from the end so earlier offsets stay valid.
    let changed = 0;
    for (let i = blocks.length - 1; i >= 0; i--) {
      const b = blocks[i];
      const bodyStart = i === 0 ? Math.max(b.start, fm) : b.start;
      if (bodyStart >= b.end) continue;
      const slide = lines.slice(bodyStart, b.end).join("\n");
      if (extractNotesFromSlideMd(slide)) continue;
      const draft = draftNotesFromSlideMd(slide);
      const noteBlock = `<!-- notes:\n${draft}\n-->`;
      const next = slide.replace(/\s*$/, "\n\n" + noteBlock + "\n");
      lines.splice(bodyStart, b.end - bodyStart, ...next.split("\n"));
      changed++;
    }
    if (!changed) {
      toastMsg("Every slide already has notes");
      return;
    }
    $("editor").value = lines.join("\n").replace(/\n{3,}/g, "\n\n");
    schedulePreview();
    toastMsg(`Drafted notes on ${changed} slide(s)`);
  }

  // -------------------------------------------------------------------------
  // Session fonts (OPFS) — @font-face for preview; title_font / body_font FM
  // -------------------------------------------------------------------------
  state.sessionFonts = state.sessionFonts || [];

  async function opfsFontDir(create = true) {
    if (!navigator.storage?.getDirectory) return null;
    const root = await navigator.storage.getDirectory();
    return root.getDirectoryHandle(FONT_DIR, { create });
  }

  async function listSessionFonts() {
    const dir = await opfsFontDir(false).catch(() => null);
    if (!dir) return [];
    const out = [];
    for await (const [name, handle] of dir.entries()) {
      if (handle.kind !== "file") continue;
      if (!/\.(ttf|otf|woff2?)$/i.test(name)) continue;
      out.push({ name, handle });
    }
    out.sort((a, b) => a.name.localeCompare(b.name));
    return out;
  }

  function familyFromFileName(name) {
    return name
      .replace(/\.(ttf|otf|woff2?)$/i, "")
      .replace(/[-_]+/g, " ")
      .trim() || "SessionFont";
  }

  async function fontToDataUrl(file) {
    const buf = await file.arrayBuffer();
    const bytes = new Uint8Array(buf);
    let bin = "";
    for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]);
    const b64 = btoa(bin);
    const ext = (file.name.split(".").pop() || "ttf").toLowerCase();
    const mime =
      ext === "woff2"
        ? "font/woff2"
        : ext === "woff"
          ? "font/woff"
          : ext === "otf"
            ? "font/otf"
            : "font/ttf";
    return { family: familyFromFileName(file.name), dataUrl: `data:${mime};base64,${b64}`, name: file.name };
  }

  async function loadSessionFontsIntoState() {
    const listed = await listSessionFonts();
    const fonts = [];
    for (const f of listed) {
      try {
        const file = await f.handle.getFile();
        fonts.push(await fontToDataUrl(file));
      } catch {
        /* skip broken */
      }
    }
    state.sessionFonts = fonts;
    return fonts;
  }

  async function injectSessionFonts() {
    if (!state.sessionFonts?.length) {
      await loadSessionFontsIntoState();
    }
    const css = (state.sessionFonts || [])
      .map(
        (f) =>
          `@font-face{font-family:"${f.family.replace(/"/g, "")}";src:url("${f.dataUrl}") format("${
            f.name.toLowerCase().endsWith("woff2")
              ? "woff2"
              : f.name.toLowerCase().endsWith("woff")
                ? "woff"
                : f.name.toLowerCase().endsWith("otf")
                  ? "opentype"
                  : "truetype"
          }");font-display:swap;}`
      )
      .join("\n");
    const apply = (doc) => {
      if (!doc) return;
      let el = doc.getElementById("session-fonts");
      if (!el) {
        el = doc.createElement("style");
        el.id = "session-fonts";
        doc.head?.appendChild(el);
      }
      el.textContent = css;
    };
    apply($("preview")?.contentDocument);
    apply($("ghost-frame")?.contentDocument);
  }

  async function importFontFiles(fileList) {
    const dir = await opfsFontDir(true);
    if (!dir) {
      toastMsg("OPFS not available in this browser");
      return;
    }
    let n = 0;
    for (const file of fileList) {
      if (!/\.(ttf|otf|woff2?)$/i.test(file.name)) continue;
      const handle = await dir.getFileHandle(file.name, { create: true });
      const w = await handle.createWritable();
      await w.write(await file.arrayBuffer());
      await w.close();
      n++;
    }
    await loadSessionFontsIntoState();
    await injectSessionFonts();
    toastMsg(n ? `Stored ${n} font(s) in OPFS` : "No font files");
  }

  async function openFontsPanel() {
    const wrap = document.createElement("div");
    wrap.className = "fonts-panel";
    wrap.innerHTML = `
      <p class="muted">Fonts stay on this device (OPFS). Preview uses <code>@font-face</code>.
      Set front-matter <code>title_font</code> / <code>body_font</code> to the family name for HTML/PPTX theme hooks.</p>
      <div class="brand-actions" style="margin-bottom:0.75rem;flex-wrap:wrap;gap:0.4rem">
        <button type="button" class="primary" id="font-add">Add font files…</button>
        <button type="button" id="font-refresh">Refresh</button>
      </div>
      <div id="font-list" class="font-list"></div>
    `;
    openDrawerSafe("Session fonts", wrap);

    async function renderList() {
      const fonts = await loadSessionFontsIntoState();
      await injectSessionFonts();
      const host = wrap.querySelector("#font-list");
      host.innerHTML = "";
      if (!fonts.length) {
        host.innerHTML = `<p class="muted">No session fonts yet. Add a .ttf / .otf.</p>`;
        return;
      }
      fonts.forEach((f) => {
        const row = document.createElement("div");
        row.className = "font-row";
        row.innerHTML = `
          <div class="font-sample" style="font-family:'${escapeHtml(f.family)}',system-ui">
            <strong></strong>
            <span>The quick brown fox — 0123456789</span>
          </div>
          <div class="brand-actions" style="flex-wrap:wrap;gap:0.35rem">
            <button type="button" data-act="title">Use as title_font</button>
            <button type="button" data-act="body">Use as body_font</button>
            <button type="button" data-act="del">Remove</button>
          </div>`;
        row.querySelector("strong").textContent = f.family;
        row.querySelector('[data-act="title"]').addEventListener("click", () => {
          applyFontField("title_font", f.family);
        });
        row.querySelector('[data-act="body"]').addEventListener("click", () => {
          applyFontField("body_font", f.family);
        });
        row.querySelector('[data-act="del"]').addEventListener("click", async () => {
          try {
            const dir = await opfsFontDir(false);
            if (dir) await dir.removeEntry(f.name);
          } catch {
            /* ignore */
          }
          await renderList();
          toastMsg("Removed " + f.name);
        });
        host.appendChild(row);
      });
    }

    function applyFontField(key, family) {
      const ta = $("editor");
      const { end, lines } = parseFrontMatter(ta.value);
      const quoted = `"${family.replace(/"/g, "")}"`;
      if (end < 0) {
        ta.value = `---\n${key}: ${quoted}\n---\n\n` + ta.value;
      } else {
        let found = false;
        for (let i = 1; i < end; i++) {
          if (new RegExp(`^${key}\\s*:`, "i").test(lines[i])) {
            lines[i] = `${key}: ${quoted}`;
            found = true;
            break;
          }
        }
        if (!found) lines.splice(end, 0, `${key}: ${quoted}`);
        ta.value = lines.join("\n");
      }
      schedulePreview();
      injectSessionFonts();
      toastMsg(`${key} → ${family}`);
    }

    wrap.querySelector("#font-add").addEventListener("click", () => {
      $("#font-input")?.click();
    });
    wrap.querySelector("#font-refresh").addEventListener("click", () => renderList());
    await renderList();
  }

  // -------------------------------------------------------------------------
  // Wire
  // -------------------------------------------------------------------------
  function wire() {
    $("#btn-ghost")?.addEventListener("click", () => toggleGhost());
    $("#btn-notes")?.addEventListener("click", () => openNotesPanel());
    $("#btn-fonts")?.addEventListener("click", () => openFontsPanel());
    $("#btn-ai")?.addEventListener("click", () => openAiChat());
    $("#btn-git")?.addEventListener("click", () => openGitHelper());
    document.querySelectorAll("[data-ghost-mode]").forEach((btn) => {
      btn.addEventListener("click", (e) => {
        e.stopPropagation();
        setGhostMode(btn.getAttribute("data-ghost-mode"));
      });
    });
    $("#font-input")?.addEventListener("change", async (e) => {
      const files = e.target.files;
      if (files?.length) {
        await importFontFiles(files);
        // refresh open panel if present
        if ($("#drawer") && !$("#drawer").hidden && $("#drawer-title")?.textContent === "Session fonts") {
          openFontsPanel();
        }
      }
      e.target.value = "";
    });
    syncGhostModeButtons();
    loadSessionFontsIntoState().then(() => injectSessionFonts()).catch(() => {});

    window.addEventListener("md2any-palette-extra", (ev) => {
      const cmds = ev.detail?.cmds;
      if (!cmds) return;
      cmds.push(...proCommands());
    });

    patchPalette();
  }

  function proCommands() {
    return [
      {
        id: "ghost",
        label: "Toggle export ghost viewport",
        run: () => toggleGhost(),
      },
      {
        id: "ghost-html",
        label: "Ghost mode → HTML export",
        run: () => {
          setGhostEnabled(true);
          setGhostMode("html");
        },
      },
      {
        id: "ghost-svg",
        label: "Ghost mode → SVG layout (PPTX-like)",
        run: () => {
          setGhostEnabled(true);
          setGhostMode("svg");
        },
      },
      {
        id: "notes",
        label: "Speaker notes panel",
        run: () => openNotesPanel(),
      },
      {
        id: "notes-all",
        label: "Draft speaker notes for all slides (IR)",
        run: () => draftNotesAllFromIr(),
      },
      {
        id: "fonts",
        label: "Session fonts (OPFS)…",
        run: () => openFontsPanel(),
      },
      {
        id: "ai-chat",
        label: "AI assistant (BYO key)",
        run: () => openAiChat(),
      },
      {
        id: "ai-settings",
        label: "AI settings…",
        run: () => openAiSettings(),
      },
      {
        id: "git",
        label: "Git commit helper",
        run: () => openGitHelper(),
      },
    ];
  }

  function patchPalette() {
    window.__studioProCommands = () => proCommands();
  }

  return {
    wire,
    scheduleGhost,
    toggleGhost,
    openAiChat,
    openAiSettings,
    openGitHelper,
    openNotesPanel,
    openFontsPanel,
    injectSessionFonts,
    paintFilmstripThumbs,
    onPreviewApplied: () => {
      scheduleGhost(1400);
      scheduleFilmstripThumbs(1600);
      injectSessionFonts().catch(() => {});
      paintFilmstripThumbs();
    },
  };
}
