//! Minimal preview server for `--serve`. No deps beyond std.
//!
//! - Builds the deck once on startup.
//! - Spawns a watcher thread that re-renders when any input file's mtime
//!   changes (same model as `--watch`).
//! - Serves a localhost-only preview shell plus the selected artifact:
//!     - `GET /`              -> HTML preview shell, polls /version
//!     - `GET /deck.pdf`      -> current PDF bytes in PDF mode
//!     - `GET /deck.html`     -> current standalone HTML in HTML mode
//!     - `GET /slides/NNN.*`  -> current SVG/PNG slide bytes in image mode
//!     - `GET /manifest.json` -> format, slide count, version, error
//!     - `GET /version`       -> integer incremented on every rebuild
//!
//! Designed for the common dev loop: edit in your editor, save, see the
//! browser tab refresh automatically.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

pub struct ServeOpts {
    pub port: u16,
    pub bind: String,
    /// Serve the in-browser editor (edit markdown + live preview side by side)
    /// instead of the preview-only shell. Edits are written back to the source
    /// file, which the watcher rebuilds.
    pub edit: bool,
}

impl Default for ServeOpts {
    fn default() -> Self {
        ServeOpts {
            port: 8421,
            bind: "127.0.0.1".into(),
            edit: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServeFormat {
    Pdf,
    Html,
    Svg,
    Png,
}

impl ServeFormat {
    pub fn name(self) -> &'static str {
        match self {
            ServeFormat::Pdf => "pdf",
            ServeFormat::Html => "html",
            ServeFormat::Svg => "svg",
            ServeFormat::Png => "png",
        }
    }

    fn image_content_type(self) -> &'static str {
        match self {
            ServeFormat::Svg => "image/svg+xml; charset=utf-8",
            ServeFormat::Png => "image/png",
            _ => "application/octet-stream",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ServedFile {
    pub name: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub enum ServedArtifact {
    Pdf(Vec<u8>),
    Html(Vec<u8>),
    Images {
        format: ServeFormat,
        files: Vec<ServedFile>,
    },
}

impl ServedArtifact {
    fn format(&self) -> ServeFormat {
        match self {
            ServedArtifact::Pdf(_) => ServeFormat::Pdf,
            ServedArtifact::Html(_) => ServeFormat::Html,
            ServedArtifact::Images { format, .. } => *format,
        }
    }

    fn slide_count(&self) -> usize {
        match self {
            ServedArtifact::Pdf(_) | ServedArtifact::Html(_) => 0,
            ServedArtifact::Images { files, .. } => files.len(),
        }
    }
}

struct State {
    artifact: ServedArtifact,
    version: u64,
    error: Option<String>,
    /// When the editor is enabled, the source file that `GET/POST /source`
    /// reads and writes (the first input). `None` in preview-only mode.
    edit_source: Option<PathBuf>,
}

pub fn run<F>(
    opts: ServeOpts,
    paths: Vec<PathBuf>,
    format: ServeFormat,
    mut build: F,
) -> std::io::Result<()>
where
    F: FnMut() -> Result<ServedArtifact, String> + Send + 'static,
{
    // The editor reads/writes the first input file. Multi-file concat decks
    // stay preview-only (editing a concatenation is ambiguous).
    let edit_source = if opts.edit {
        paths.first().cloned()
    } else {
        None
    };
    let initial = match build() {
        Ok(artifact) => State {
            artifact,
            version: 1,
            error: None,
            edit_source: edit_source.clone(),
        },
        Err(e) => {
            eprintln!("md2any: initial build failed: {e}");
            State {
                artifact: error_artifact(format, &e),
                version: 1,
                error: Some(e),
                edit_source: edit_source.clone(),
            }
        }
    };
    let state = Arc::new(Mutex::new(initial));

    let watcher_state = state.clone();
    std::thread::spawn(move || {
        let fingerprint = |paths: &[PathBuf]| -> Vec<Option<SystemTime>> {
            paths
                .iter()
                .map(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
                .collect()
        };
        let mut last = fingerprint(&paths);
        loop {
            std::thread::sleep(Duration::from_millis(250));
            let now = fingerprint(&paths);
            if now != last {
                last = now;
                std::thread::sleep(Duration::from_millis(80));
                match build() {
                    Ok(artifact) => {
                        let mut s = watcher_state.lock().unwrap();
                        s.artifact = artifact;
                        s.version += 1;
                        s.error = None;
                        eprintln!("md2any: rebuilt (v{})", s.version);
                    }
                    Err(e) => {
                        let mut s = watcher_state.lock().unwrap();
                        s.error = Some(e.clone());
                        s.version += 1;
                        s.artifact = error_artifact(format, &e);
                        eprintln!("md2any: error: {e}");
                    }
                }
            }
        }
    });

    let addr = format!("{}:{}", opts.bind, opts.port);
    let listener = TcpListener::bind(&addr)?;
    eprintln!("md2any: preview at http://{addr} (Ctrl-C to stop)");

    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let state = state.clone();
        std::thread::spawn(move || {
            let _ = handle(stream, state);
        });
    }
    Ok(())
}

fn handle(mut stream: TcpStream, state: Arc<Mutex<State>>) -> std::io::Result<()> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let (method, raw_path, body) = match read_request(&mut stream) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };
    let path = raw_path.split('?').next().unwrap_or("/");

    match (method.as_str(), path) {
        ("GET", "/") => {
            let html = if state.lock().unwrap().edit_source.is_some() {
                EDITOR_HTML
            } else {
                INDEX_HTML
            };
            write_response(
                &mut stream,
                200,
                "OK",
                "text/html; charset=utf-8",
                html.as_bytes(),
            )?;
        }
        ("GET", "/version") => {
            let v = state.lock().unwrap().version;
            let body = v.to_string();
            write_response(&mut stream, 200, "OK", "text/plain", body.as_bytes())?;
        }
        ("GET", "/manifest.json") => {
            let s = state.lock().unwrap();
            let body = manifest_json(&s);
            write_response(
                &mut stream,
                200,
                "OK",
                "application/json; charset=utf-8",
                body.as_bytes(),
            )?;
        }
        ("GET", "/deck.pdf") => {
            let s = state.lock().unwrap();
            if let ServedArtifact::Pdf(bytes) = &s.artifact {
                let bytes = bytes.clone();
                drop(s);
                write_response(&mut stream, 200, "OK", "application/pdf", &bytes)?;
            } else {
                write_response(&mut stream, 404, "Not Found", "text/plain", b"not found")?;
            }
        }
        ("GET", "/deck.html") => {
            let s = state.lock().unwrap();
            if let ServedArtifact::Html(bytes) = &s.artifact {
                let bytes = bytes.clone();
                drop(s);
                write_response(&mut stream, 200, "OK", "text/html; charset=utf-8", &bytes)?;
            } else {
                write_response(&mut stream, 404, "Not Found", "text/plain", b"not found")?;
            }
        }
        // Editor: read the current source markdown.
        ("GET", "/source") => {
            let src = state.lock().unwrap().edit_source.clone();
            match src.and_then(|p| std::fs::read(&p).ok()) {
                Some(bytes) => write_response(
                    &mut stream,
                    200,
                    "OK",
                    "text/markdown; charset=utf-8",
                    &bytes,
                )?,
                None => write_response(
                    &mut stream,
                    404,
                    "Not Found",
                    "text/plain",
                    b"editing disabled",
                )?,
            }
        }
        // Editor: write edited markdown back to the source file. The watcher
        // picks up the mtime change and rebuilds, bumping /version.
        ("POST", "/source") => {
            let src = state.lock().unwrap().edit_source.clone();
            match src {
                Some(p) => match std::fs::write(&p, &body) {
                    Ok(()) => write_response(&mut stream, 200, "OK", "text/plain", b"ok")?,
                    Err(e) => {
                        let msg = format!("write {}: {e}", p.display());
                        write_response(&mut stream, 500, "Error", "text/plain", msg.as_bytes())?;
                    }
                },
                None => write_response(
                    &mut stream,
                    403,
                    "Forbidden",
                    "text/plain",
                    b"editing disabled",
                )?,
            }
        }
        ("GET", _) if path.starts_with("/slides/") => {
            let name = &path["/slides/".len()..];
            let s = state.lock().unwrap();
            if let ServedArtifact::Images { format, files } = &s.artifact {
                if let Some(file) = files.iter().find(|file| file.name == name) {
                    let bytes = file.bytes.clone();
                    let content_type = format.image_content_type();
                    drop(s);
                    write_response(&mut stream, 200, "OK", content_type, &bytes)?;
                } else {
                    write_response(&mut stream, 404, "Not Found", "text/plain", b"not found")?;
                }
            } else {
                write_response(&mut stream, 404, "Not Found", "text/plain", b"not found")?;
            }
        }
        _ => {
            write_response(&mut stream, 404, "Not Found", "text/plain", b"not found")?;
        }
    }
    Ok(())
}

/// Read a full HTTP request: returns `(method, path, body)`. Reads headers up
/// to the blank line, then `Content-Length` body bytes — so POSTs larger than
/// one packet (a whole markdown deck) arrive intact, unlike a single `read`.
fn read_request<R: Read>(stream: &mut R) -> std::io::Result<(String, String, Vec<u8>)> {
    let mut buf: Vec<u8> = Vec::with_capacity(2048);
    let mut tmp = [0u8; 4096];
    let header_end = loop {
        if let Some(pos) = find_subseq(&buf, b"\r\n\r\n") {
            break pos + 4;
        }
        let n = stream.read(&mut tmp)?;
        if n == 0 {
            break buf.len();
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.len() > 16 * 1024 * 1024 {
            break buf.len();
        }
    };
    let header_end = header_end.min(buf.len());
    let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let mut first = head.lines().next().unwrap_or("").split_whitespace();
    let method = first.next().unwrap_or("GET").to_string();
    let path = first.next().unwrap_or("/").to_string();
    let content_length = head
        .lines()
        .find_map(|l| {
            let lower = l.to_ascii_lowercase();
            lower
                .strip_prefix("content-length:")
                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
        })
        .unwrap_or(0);
    let mut body = buf[header_end..].to_vec();
    while body.len() < content_length {
        let n = stream.read(&mut tmp)?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    body.truncate(content_length.min(body.len()));
    Ok((method, path, body))
}

fn find_subseq(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn write_response(
    stream: &mut TcpStream,
    code: u16,
    status: &str,
    content_type: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let header = format!(
        "HTTP/1.1 {code} {status}\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {len}\r\n\
         Cache-Control: no-store\r\n\
         Connection: close\r\n\r\n",
        len = body.len(),
    );
    stream.write_all(header.as_bytes())?;
    stream.write_all(body)?;
    Ok(())
}

const INDEX_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8"/>
<title>md2any preview</title>
<style>
  html, body { margin: 0; height: 100%; background: #1a1a1a; color: #ddd; font-family: system-ui, sans-serif; }
  embed, iframe { width: 100%; height: 100%; border: 0; background: white; }
  #stage { width: 100%; height: 100%; display: grid; place-items: center; }
  #stage img { max-width: 100%; max-height: 100%; object-fit: contain; background: white; }
  #error {
    display: none; position: fixed; left: 16px; right: 16px; top: 16px;
    padding: 12px 14px; background: #7f1d1d; color: white;
    font-size: 13px; border-radius: 4px; z-index: 4;
  }
  #controls {
    display: none; position: fixed; left: 50%; bottom: 12px; transform: translateX(-50%);
    align-items: center; gap: 8px; padding: 6px 8px;
    background: rgba(17,24,39,.84); color: white; border-radius: 4px;
    font-size: 13px;
  }
  #controls button {
    width: 30px; height: 28px; border: 0; border-radius: 3px;
    background: rgba(255,255,255,.16); color: white; cursor: pointer;
  }
  #toast {
    position: fixed; top: 12px; right: 12px;
    padding: 6px 12px; background: #2563eb; color: white;
    font-size: 13px; border-radius: 4px; opacity: 0;
    transition: opacity .2s; pointer-events: none;
  }
  #toast.show { opacity: 1; }
</style>
</head>
<body>
<div id="stage"></div>
<div id="error"></div>
<div id="controls">
  <button type="button" id="prev" aria-label="Previous slide">&lsaquo;</button>
  <span><b id="current">1</b>/<span id="total">1</span></span>
  <button type="button" id="next" aria-label="Next slide">&rsaquo;</button>
</div>
<div id="toast">reloading…</div>
<script>
let known = null;
let manifest = null;
let slide = 1;
const toast = document.getElementById('toast');
const stage = document.getElementById('stage');
const errorBox = document.getElementById('error');
const controls = document.getElementById('controls');
const current = document.getElementById('current');
const total = document.getElementById('total');
function slideName(n, format) {
  return '/slides/slide-' + String(n).padStart(3, '0') + '.' + format;
}
function showError(message) {
  if (message) {
    errorBox.textContent = message;
    errorBox.style.display = 'block';
  } else {
    errorBox.textContent = '';
    errorBox.style.display = 'none';
  }
}
function render() {
  if (!manifest) return;
  showError(manifest.error || '');
  controls.style.display = 'none';
  if (manifest.format === 'pdf') {
    stage.innerHTML = '<embed id="pdf" src="/deck.pdf?v=' + manifest.version + '" type="application/pdf"/>';
  } else if (manifest.format === 'html') {
    stage.innerHTML = '<iframe id="html" src="/deck.html?v=' + manifest.version + '" title="md2any HTML preview"></iframe>';
  } else if (manifest.format === 'svg' || manifest.format === 'png') {
    const count = Math.max(1, manifest.slide_count || 1);
    slide = Math.max(1, Math.min(count, slide));
    controls.style.display = 'flex';
    current.textContent = String(slide);
    total.textContent = String(count);
    stage.innerHTML = '<img id="slide" src="' + slideName(slide, manifest.format) + '?v=' + manifest.version + '" alt="Slide ' + slide + '"/>';
  } else {
    stage.textContent = 'Unsupported preview format: ' + manifest.format;
  }
}
async function loadManifest() {
  const r = await fetch('/manifest.json', { cache: 'no-store' });
  manifest = await r.json();
  known = String(manifest.version);
  render();
}
async function tick() {
  try {
    const r = await fetch('/version', { cache: 'no-store' });
    const v = (await r.text()).trim();
    if (known === null) {
      known = v;
    } else if (v !== known) {
      known = v;
      toast.classList.add('show');
      await loadManifest();
      setTimeout(() => toast.classList.remove('show'), 800);
    }
  } catch (e) {}
}
document.getElementById('prev').addEventListener('click', () => { slide--; render(); });
document.getElementById('next').addEventListener('click', () => { slide++; render(); });
window.addEventListener('keydown', (event) => {
  if (!manifest || (manifest.format !== 'svg' && manifest.format !== 'png')) return;
  if (['ArrowRight', 'PageDown', ' ', 'Enter'].includes(event.key)) {
    event.preventDefault();
    slide++;
    render();
  } else if (['ArrowLeft', 'PageUp', 'Backspace'].includes(event.key)) {
    event.preventDefault();
    slide--;
    render();
  } else if (event.key === 'Home') {
    event.preventDefault();
    slide = 1;
    render();
  } else if (event.key === 'End') {
    event.preventDefault();
    slide = manifest.slide_count || 1;
    render();
  }
});
setInterval(tick, 500);
loadManifest().catch(() => showError('unable to load md2any preview manifest'));
</script>
</body>
</html>
"#;

const EDITOR_HTML: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8"/>
<title>md2any editor</title>
<style>
  html, body { margin: 0; height: 100%; font-family: system-ui, sans-serif; }
  body { display: flex; flex-direction: column; background: #0f172a; color: #e5e7eb; }
  #bar {
    display: flex; align-items: center; gap: 12px; padding: 8px 12px;
    background: #111827; border-bottom: 1px solid #1f2937; font-size: 13px; flex: 0 0 auto;
  }
  #bar b { color: #fff; }
  #status { color: #93c5fd; }
  #status.err { color: #fca5a5; }
  #fmt { margin-left: auto; color: #9ca3af; }
  #main { flex: 1 1 auto; display: flex; min-height: 0; }
  #edit { width: 44%; min-width: 220px; display: flex; }
  #src {
    width: 100%; border: 0; resize: none; outline: none;
    background: #0b1220; color: #e5e7eb; padding: 14px;
    font: 13px/1.55 ui-monospace, "DejaVu Sans Mono", monospace;
    tab-size: 2; white-space: pre; overflow: auto;
  }
  #divider { width: 1px; background: #1f2937; }
  /* The preview is a scrollable strip of per-slide images we own, so a rebuild
     swaps image sources in place (scroll preserved) instead of reloading a
     whole document and snapping to the top. */
  #view { flex: 1 1 auto; overflow: auto; background: #1a1a1a; min-width: 0; }
  #error {
    display: none; position: sticky; top: 0; margin: 8px;
    padding: 10px 12px; background: #7f1d1d; color: #fff; font-size: 12px;
    border-radius: 4px; z-index: 4; white-space: pre-wrap;
  }
  #deck { display: flex; flex-direction: column; align-items: center; gap: 14px; padding: 16px; }
  .pslide {
    width: 100%; max-width: 980px; border-radius: 2px; outline: 2px solid transparent;
    outline-offset: 3px; transition: outline-color .15s; box-shadow: 0 2px 10px rgba(0,0,0,.45);
  }
  .pslide.active { outline-color: #2563eb; }
  .pslide img { width: 100%; display: block; background: #fff; }
  #deck > embed, #deck > iframe { width: 100%; height: 80vh; border: 0; background: #fff; }
</style>
</head>
<body>
<div id="bar"><b>md2any</b> editor <span id="status">ready</span><span id="fmt"></span></div>
<div id="main">
  <div id="edit"><textarea id="src" spellcheck="false" placeholder="# Type markdown here…"></textarea></div>
  <div id="divider"></div>
  <div id="view">
    <div id="error"></div>
    <div id="deck"></div>
  </div>
</div>
<script>
const ta = document.getElementById('src');
const statusEl = document.getElementById('status');
const fmtEl = document.getElementById('fmt');
const deck = document.getElementById('deck');
const errorBox = document.getElementById('error');
let known = null, ver = 0, fmt = 'svg', count = 0, isImage = true, saveTimer = null;

function setStatus(t, err) { statusEl.textContent = t; statusEl.className = err ? 'err' : ''; }
function showError(m) { errorBox.style.display = m ? 'block' : 'none'; errorBox.textContent = m || ''; }
function srcFor(i) { return '/slides/slide-' + String(i + 1).padStart(3, '0') + '.' + fmt + '?v=' + ver; }

// Match the DOM slide-count, reusing existing nodes so scroll is preserved.
function ensureSlides(n) {
  while (deck.children.length < n) {
    const d = document.createElement('div');
    d.className = 'pslide';
    const img = document.createElement('img');
    img.alt = 'Slide ' + (deck.children.length + 1);
    d.appendChild(img);
    deck.appendChild(d);
  }
  while (deck.children.length > n) deck.removeChild(deck.lastChild);
}
function refreshSrcs() {
  for (let i = 0; i < deck.children.length; i++) {
    const img = deck.children[i].querySelector('img');
    if (!img) continue;
    const s = srcFor(i);
    if (img.getAttribute('src') !== s) img.setAttribute('src', s);
  }
}
// Map the caret to a slide: count slide-start markers (#, ##, --- rule) before
// it, skipping a leading front-matter block. Approximate (the paginator may add
// continuation slides), but enough to keep the edited slide in view.
function activeIndex() {
  const before = ta.value.slice(0, ta.selectionStart);
  const lines = before.split('\n');
  let start = 0;
  if (lines[0] && lines[0].trim() === '---') {
    for (let j = 1; j < lines.length; j++) {
      if (lines[j].trim() === '---') { start = j + 1; break; }
    }
  }
  let idx = 0;
  for (let j = start; j < lines.length; j++) {
    if (/^#{1,2}\s/.test(lines[j]) || /^---\s*$/.test(lines[j])) idx++;
  }
  const zero = ta.value.startsWith('---') ? idx : Math.max(0, idx - 1);
  return Math.min(Math.max(zero, 0), Math.max(0, count - 1));
}
function focusActive(scroll) {
  if (!isImage || count === 0) return;
  const i = activeIndex();
  for (let k = 0; k < deck.children.length; k++) {
    deck.children[k].classList.toggle('active', k === i);
  }
  if (scroll && deck.children[i]) deck.children[i].scrollIntoView({ block: 'nearest' });
}
async function loadManifest() {
  const m = await (await fetch('/manifest.json', { cache: 'no-store' })).json();
  ver = m.version; known = String(m.version); fmtEl.textContent = m.format;
  showError(m.error || '');
  if (m.format === 'svg' || m.format === 'png') {
    isImage = true; fmt = m.format; count = Math.max(1, m.slide_count || 1);
    ensureSlides(count); refreshSrcs(); focusActive(true);
  } else {
    // pdf/html can't be focused per slide — single embed, reloaded on change.
    isImage = false; count = 0;
    deck.innerHTML = m.format === 'pdf'
      ? '<embed src="/deck.pdf?v=' + ver + '" type="application/pdf"/>'
      : '<iframe src="/deck.html?v=' + ver + '" title="preview"></iframe>';
  }
}
async function tick() {
  try {
    const v = (await (await fetch('/version', { cache: 'no-store' })).text()).trim();
    if (known === null) known = v;
    else if (v !== known) { known = v; await loadManifest(); }
  } catch (e) {}
}
async function save() {
  setStatus('saving…');
  try {
    const r = await fetch('/source', { method: 'POST', headers: { 'Content-Type': 'text/plain; charset=utf-8' }, body: ta.value });
    setStatus(r.ok ? 'saved' : 'save failed', !r.ok);
  } catch (e) { setStatus('save failed', true); }
}
ta.addEventListener('input', () => { setStatus('editing…'); focusActive(true); clearTimeout(saveTimer); saveTimer = setTimeout(save, 450); });
ta.addEventListener('keyup', () => focusActive(true));
ta.addEventListener('click', () => focusActive(true));
ta.addEventListener('keydown', (e) => {
  if (e.key === 'Tab') {
    e.preventDefault();
    const s = ta.selectionStart, en = ta.selectionEnd;
    ta.value = ta.value.slice(0, s) + '  ' + ta.value.slice(en);
    ta.selectionStart = ta.selectionEnd = s + 2;
  }
});
fetch('/source', { cache: 'no-store' }).then(r => r.text()).then(t => { ta.value = t; }).catch(() => {});
setInterval(tick, 500);
loadManifest().catch(() => showError('unable to load preview manifest'));
</script>
</body>
</html>
"##;

fn manifest_json(state: &State) -> String {
    let error = state
        .error
        .as_ref()
        .map(|e| format!("\"{}\"", escape_json(e)))
        .unwrap_or_else(|| "null".to_string());
    format!(
        "{{\"schema\":\"md2any-serve-manifest-v1\",\"version\":{},\"format\":\"{}\",\"slide_count\":{},\"error\":{}}}",
        state.version,
        state.artifact.format().name(),
        state.artifact.slide_count(),
        error
    )
}

fn error_artifact(format: ServeFormat, message: &str) -> ServedArtifact {
    match format {
        ServeFormat::Pdf => ServedArtifact::Pdf(error_pdf(message)),
        ServeFormat::Html => ServedArtifact::Html(error_html(message).into_bytes()),
        ServeFormat::Svg | ServeFormat::Png => ServedArtifact::Images {
            format,
            files: Vec::new(),
        },
    }
}

fn error_html(message: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>md2any error</title></head><body><h1>md2any: build error</h1><pre>{}</pre></body></html>",
        escape_html(message)
    )
}

fn escape_json(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Tiny one-page PDF with an error banner so the browser still shows
/// something useful when the rebuild fails.
fn error_pdf(message: &str) -> Vec<u8> {
    let safe = message
        .replace('(', "[")
        .replace(')', "]")
        .replace('\\', "/");
    let truncated: String = safe.chars().take(180).collect();
    let content = format!(
        "BT /F1 18 Tf 0.8 0.15 0.15 rg 50 750 Td (md2any: build error) Tj ET\nBT /F1 12 Tf 0.3 0.3 0.3 rg 50 720 Td ({}) Tj ET\n",
        truncated,
    );
    let stream_obj = format!(
        "<< /Length {} >>\nstream\n{}endstream",
        content.len(),
        content
    );
    let _ = stream_obj;
    let mut buf = Vec::new();
    let _ = write!(
        &mut buf,
        "%PDF-1.4\n\
         1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n\
         2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n\
         3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >> endobj\n\
         4 0 obj << /Length {len} >> stream\n{content}endstream endobj\n\
         5 0 obj << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> endobj\n\
         xref\n0 6\n0000000000 65535 f\n0000000009 00000 n\n0000000058 00000 n\n0000000111 00000 n\n0000000234 00000 n\n0000000300 00000 n\n\
         trailer << /Size 6 /Root 1 0 R >>\nstartxref\n400\n%%EOF\n",
        len = content.len(),
        content = content,
    );
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_reports_image_sequence_details() {
        let state = State {
            artifact: ServedArtifact::Images {
                format: ServeFormat::Svg,
                files: vec![
                    ServedFile {
                        name: "slide-001.svg".into(),
                        bytes: b"<svg/>".to_vec(),
                    },
                    ServedFile {
                        name: "slide-002.svg".into(),
                        bytes: b"<svg/>".to_vec(),
                    },
                ],
            },
            version: 7,
            error: None,
            edit_source: None,
        };

        let json = manifest_json(&state);
        assert!(json.contains("\"schema\":\"md2any-serve-manifest-v1\""));
        assert!(json.contains("\"version\":7"));
        assert!(json.contains("\"format\":\"svg\""));
        assert!(json.contains("\"slide_count\":2"));
        assert!(json.contains("\"error\":null"));
    }

    #[test]
    fn read_request_parses_method_path_and_body() {
        // POST with a body split conceptually across the buffer boundary.
        let raw = b"POST /source?v=1 HTTP/1.1\r\nHost: x\r\nContent-Length: 11\r\n\r\nhello world";
        let mut cursor = std::io::Cursor::new(raw.to_vec());
        let (method, path, body) = read_request(&mut cursor).unwrap();
        assert_eq!(method, "POST");
        assert_eq!(path, "/source?v=1");
        assert_eq!(body, b"hello world");

        let raw = b"GET / HTTP/1.1\r\n\r\n";
        let mut cursor = std::io::Cursor::new(raw.to_vec());
        let (method, path, body) = read_request(&mut cursor).unwrap();
        assert_eq!(method, "GET");
        assert_eq!(path, "/");
        assert!(body.is_empty());
    }

    #[test]
    fn manifest_escapes_build_errors() {
        let state = State {
            artifact: ServedArtifact::Html(Vec::new()),
            version: 2,
            error: Some("bad \"quote\"\nnext".into()),
            edit_source: None,
        };

        let json = manifest_json(&state);
        assert!(json.contains("\"format\":\"html\""));
        assert!(json.contains("bad \\\"quote\\\"\\nnext"), "{json}");
    }
}
