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
//!
//! The `--edit` shell goes further: it previews the deck as live HTML in an
//! iframe and *morphs* each rebuild into the existing DOM (see `EDITOR_HTML`),
//! so only the changed slide's nodes update — scroll position is preserved and
//! the slide under the caret is highlighted and scrolled into view using the
//! per-slide `data-line` attributes the HTML renderer emits in editor mode.

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

/// A single rendered file produced on demand by the editor's "Generate to…"
/// menu (`GET /export?format=…`).
pub struct ExportFile {
    pub filename: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

/// Renders the current source to the requested single-file format. Called from
/// request-handler threads, so it must be `Send + Sync`.
pub type Exporter = Arc<dyn Fn(&str) -> Result<ExportFile, String> + Send + Sync>;

/// Handles an AI-dock chat turn: takes the raw `POST /chat` request body
/// (JSON `{messages:[…]}`) and streams the assistant's reply by invoking the
/// `on_delta` sink with each text fragment as it arrives. Called from
/// request-handler threads, so it must be `Send + Sync`.
pub type ChatHandler = Arc<
    dyn Fn(&str, &mut dyn FnMut(&str) -> Result<(), String>) -> Result<(), String> + Send + Sync,
>;

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
    /// Renders the current source to a requested format for `GET /export`.
    export: Exporter,
    /// Handles `POST /chat` turns for the AI dock.
    chat: ChatHandler,
}

pub fn run<F>(
    opts: ServeOpts,
    paths: Vec<PathBuf>,
    format: ServeFormat,
    mut build: F,
    export: Exporter,
    chat: ChatHandler,
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
            export: export.clone(),
            chat: chat.clone(),
        },
        Err(e) => {
            eprintln!("md2any: initial build failed: {e}");
            State {
                artifact: error_artifact(format, &e),
                version: 1,
                error: Some(e),
                edit_source: edit_source.clone(),
                export: export.clone(),
                chat: chat.clone(),
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
        // Editor "Generate to…": render the current source to a single-file
        // format and stream it back as a download.
        ("GET", "/export") => {
            let fmt = query_param(&raw_path, "format").unwrap_or_default();
            let export = state.lock().unwrap().export.clone();
            match export(&fmt) {
                Ok(file) => {
                    write_download(&mut stream, &file.content_type, &file.filename, &file.bytes)?
                }
                Err(e) => {
                    write_response(&mut stream, 400, "Bad Request", "text/plain", e.as_bytes())?
                }
            }
        }
        // AI dock: one chat turn, streamed. Body is `{messages:[…]}`; the
        // response is newline-delimited JSON — `{"delta":"…"}` per fragment,
        // then `{"done":true}` or `{"error":"…"}`.
        ("POST", "/chat") => {
            let chat = state.lock().unwrap().chat.clone();
            let body_str = String::from_utf8_lossy(&body).into_owned();
            // Flush each fragment promptly so the browser shows it streaming.
            let _ = stream.set_nodelay(true);
            stream.write_all(
                b"HTTP/1.1 200 OK\r\n\
                  Content-Type: application/x-ndjson; charset=utf-8\r\n\
                  Cache-Control: no-store\r\n\
                  Connection: close\r\n\r\n",
            )?;
            // Scope the sink so its &mut borrow of `stream` ends before the
            // final status line is written.
            let result = {
                let mut sink = |delta: &str| -> Result<(), String> {
                    let line = format!("{{\"delta\":\"{}\"}}\n", escape_json(delta));
                    stream
                        .write_all(line.as_bytes())
                        .and_then(|_| stream.flush())
                        .map_err(|e| e.to_string())
                };
                chat(&body_str, &mut sink)
            };
            match result {
                Ok(()) => {
                    let _ = stream.write_all(b"{\"done\":true}\n");
                }
                Err(e) => {
                    let _ = stream
                        .write_all(format!("{{\"error\":\"{}\"}}\n", escape_json(&e)).as_bytes());
                }
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

/// Pull a single query-string value out of a raw request path, e.g.
/// `query_param("/export?format=pdf", "format") == Some("pdf")`. Only handles
/// the plain `key=value` form the editor sends — no percent-decoding.
fn query_param(raw_path: &str, key: &str) -> Option<String> {
    let query = raw_path.split('?').nth(1)?;
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| v.to_string())
    })
}

fn write_download(
    stream: &mut TcpStream,
    content_type: &str,
    filename: &str,
    bytes: &[u8],
) -> std::io::Result<()> {
    // Strip anything that could break out of the quoted filename.
    let safe: String = filename
        .chars()
        .filter(|c| !matches!(c, '"' | '\\' | '\r' | '\n'))
        .collect();
    let header = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {len}\r\n\
         Content-Disposition: attachment; filename=\"{safe}\"\r\n\
         Cache-Control: no-store\r\n\
         Connection: close\r\n\r\n",
        len = bytes.len(),
    );
    stream.write_all(header.as_bytes())?;
    stream.write_all(bytes)?;
    Ok(())
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
  #pos { color: #6b7280; }
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
  /* Live-DOM preview: the iframe holds the rendered deck and we morph each
     rebuild into its existing DOM, so only the changed slide's nodes update —
     scroll position and image-load state on every other slide are preserved. */
  #view { flex: 1 1 auto; display: flex; flex-direction: column; min-width: 0; background: #1a1a1a; }
  #error {
    display: none; margin: 8px;
    padding: 10px 12px; background: #7f1d1d; color: #fff; font-size: 12px;
    border-radius: 4px; z-index: 4; white-space: pre-wrap; flex: 0 0 auto;
  }
  #host { flex: 1 1 auto; min-height: 0; overflow: auto; }
  #host iframe, #host embed { width: 100%; height: 100%; border: 0; background: #fff; display: block; }
  #host img { width: 100%; display: block; background: #fff; }
  #bar button.tool { background: #1f2937; color: #e5e7eb; border: 0; border-radius: 4px; padding: 4px 9px; cursor: pointer; font-size: 12px; }
  #bar button.tool:hover { background: #374151; }
  #menu { position: relative; }
  #genMenu { display: none; position: absolute; right: 0; top: 30px; background: #0b1220; border: 1px solid #1f2937; border-radius: 6px; box-shadow: 0 8px 24px rgba(0,0,0,.45); min-width: 168px; z-index: 30; padding: 4px; }
  #genMenu.open { display: block; }
  #genMenu button { display: block; width: 100%; text-align: left; background: none; border: 0; color: #cbd5e1; padding: 6px 10px; font-size: 12px; cursor: pointer; border-radius: 4px; }
  #genMenu button:hover { background: #1f2937; }
  #genMenu .sep { height: 1px; background: #1f2937; margin: 4px 2px; }
  /* Slide-in style panel: front-matter-backed theme/colour/size controls. */
  #panel { position: fixed; top: 0; right: 0; height: 100%; width: 286px; box-sizing: border-box;
    background: #0b1220; border-left: 1px solid #1f2937; box-shadow: -8px 0 24px rgba(0,0,0,.4);
    transform: translateX(100%); transition: transform .18s ease; z-index: 20; overflow: auto; padding: 14px; }
  #panel.open { transform: none; }
  #panel h3 { margin: 16px 0 6px; font-size: 11px; text-transform: uppercase; letter-spacing: .08em; color: #6b7280; }
  #panel h3:first-child { margin-top: 0; }
  #panel .row { display: flex; align-items: center; gap: 8px; margin: 6px 0; font-size: 12px; color: #cbd5e1; }
  #panel .row label { flex: 1 1 auto; }
  #panel .swatches { display: flex; flex-wrap: wrap; gap: 6px; }
  #panel .swatch { width: 26px; height: 26px; border-radius: 4px; border: 2px solid transparent; cursor: pointer; }
  #panel .swatch.sel { border-color: #fff; }
  #panel .seg { display: flex; flex-wrap: wrap; gap: 4px; }
  #panel .seg button { background: #1f2937; color: #cbd5e1; border: 0; border-radius: 4px; padding: 3px 8px; font-size: 12px; cursor: pointer; }
  #panel .seg button.sel { background: #2563eb; color: #fff; }
  #panel input[type=range] { flex: 1 1 auto; min-width: 0; }
  #panel input[type=color] { width: 34px; height: 24px; border: 0; background: none; padding: 0; cursor: pointer; }
  #panel .val { width: 30px; text-align: right; color: #9ca3af; }
  #panel .clear { background: none; border: 0; color: #6b7280; cursor: pointer; font-size: 13px; padding: 0 2px; }
  #panel .clear:hover { color: #e5e7eb; }
  #panel input[type=text] { width: 100%; background: #111827; color: #e5e7eb; border: 1px solid #1f2937; border-radius: 4px; padding: 5px 7px; font-size: 12px; box-sizing: border-box; }
  /* AI assistant dock (bottom). */
  #ai { flex: 0 0 auto; display: flex; flex-direction: column; border-top: 1px solid #1f2937; background: #0b1220; max-height: 44%; }
  #ai.collapsed { max-height: none; }
  #ai.collapsed #chat, #ai.collapsed #airow { display: none; }
  #aibar { display: flex; align-items: center; gap: 10px; padding: 7px 12px; background: #111827; font-size: 13px; color: #e5e7eb; cursor: pointer; flex: 0 0 auto; }
  #aibar #aihint { color: #6b7280; font-size: 12px; }
  #aibar button { margin-left: auto; }
  #chat { flex: 1 1 auto; overflow: auto; padding: 12px; display: flex; flex-direction: column; gap: 9px; min-height: 140px; }
  .msg { max-width: 82%; padding: 8px 11px; border-radius: 11px; font-size: 13px; line-height: 1.46; white-space: pre-wrap; word-break: break-word; }
  .msg.user { align-self: flex-end; background: #2563eb; color: #fff; }
  .msg.bot { align-self: flex-start; background: #1f2937; color: #e5e7eb; }
  .msg.err { align-self: flex-start; background: #7f1d1d; color: #fff; }
  .msg.busy { opacity: .6; font-style: italic; }
  .msg .apply { margin-top: 9px; }
  .msg .apply button { background: #16a34a; color: #fff; border: 0; border-radius: 6px; padding: 6px 11px; font-size: 12px; cursor: pointer; }
  .msg .apply button:disabled { background: #374151; cursor: default; }
  #aichips { display: flex; flex-wrap: wrap; gap: 6px; padding: 0 12px 8px; flex: 0 0 auto; }
  #ai.collapsed #aichips { display: none; }
  .chip { background: #1f2937; color: #cbd5e1; border: 1px solid #374151; border-radius: 999px; padding: 4px 10px; font-size: 12px; cursor: pointer; }
  .chip:hover { background: #374151; color: #fff; }
  #activeChip { display: none; align-items: center; gap: 8px; margin: 0 12px 6px; padding: 4px 10px; background: #0c2540; border: 1px solid #1d4ed8; border-radius: 6px; font-size: 12px; color: #bfdbfe; }
  #activeChip b { color: #fff; }
  #activeChip button { margin-left: auto; background: none; border: 0; color: #93c5fd; cursor: pointer; font-size: 13px; }
  #airow { display: flex; gap: 8px; padding: 9px 12px; border-top: 1px solid #1f2937; flex: 0 0 auto; }
  #aiInput { flex: 1 1 auto; resize: none; background: #0b1220; color: #e5e7eb; border: 1px solid #1f2937; border-radius: 6px; padding: 8px 10px; font: 13px/1.4 system-ui, sans-serif; }
  #aiInput:focus { outline: none; border-color: #2563eb; }
</style>
</head>
<body>
<div id="bar"><b>md2any</b> editor <span id="status">ready</span><span id="pos"></span><span id="fmt"></span><div id="menu"><button class="tool" id="genBtn">Generate &#9662;</button><div id="genMenu">
  <button data-md>Save .md</button>
  <div class="sep"></div>
  <button data-fmt="pptx">PowerPoint (.pptx)</button>
  <button data-fmt="odp">Impress (.odp)</button>
  <button data-fmt="pdf">PDF (.pdf)</button>
  <button data-fmt="docx">Word (.docx)</button>
  <button data-fmt="odt">Writer (.odt)</button>
  <button data-fmt="html">HTML (.html)</button>
</div></div><button class="tool" id="styleBtn" title="Style panel">🎨 Style</button></div>
<div id="main">
  <div id="edit"><textarea id="src" spellcheck="false" placeholder="# Type markdown here…"></textarea></div>
  <div id="divider"></div>
  <div id="view">
    <div id="error"></div>
    <div id="host"></div>
  </div>
</div>
<div id="ai" class="collapsed">
  <div id="aibar"><span>🤖 Assistant</span><span id="aihint">ask about or edit this deck — it can see your markdown</span><button class="tool" id="aiToggle" title="Toggle assistant">&#9650;</button></div>
  <div id="chat"></div>
  <div id="aichips">
    <button class="chip">Proofread the whole deck</button>
    <button class="chip">Make it more concise</button>
    <button class="chip">Add a summary slide at the end</button>
    <button class="chip">Add speaker notes to each slide</button>
    <button class="chip">Suggest a better title</button>
  </div>
  <div id="activeChip"><span>✏️ Editing</span> <b id="activeChipLabel"></b><button id="activeChipX" title="Clear selection">✕</button></div>
  <div id="airow">
    <textarea id="aiInput" rows="1" placeholder="click a slide to target it, then ask — e.g. “add a takeaway here” or “add a diagram on the left”…"></textarea>
    <button class="tool" id="aiSend">Send</button>
  </div>
</div>
<div id="panel">
  <h3>Theme</h3>
  <div class="swatches" id="themes"></div>
  <h3>Aspect</h3>
  <div class="seg" id="aspect"></div>
  <h3>Transition</h3>
  <div class="seg" id="transition"></div>
  <h3>Colours</h3>
  <div class="row"><label>Accent</label><input type="color" data-skey="accent"><button class="clear" data-clear="accent" title="Reset">&#x21ba;</button></div>
  <div class="row"><label>Background</label><input type="color" data-skey="bg"><button class="clear" data-clear="bg" title="Reset">&#x21ba;</button></div>
  <div class="row"><label>Title</label><input type="color" data-skey="title_color"><button class="clear" data-clear="title_color" title="Reset">&#x21ba;</button></div>
  <div class="row"><label>Text</label><input type="color" data-skey="body_color"><button class="clear" data-clear="body_color" title="Reset">&#x21ba;</button></div>
  <h3>Type</h3>
  <div class="row"><label>Title size</label><input type="range" min="22" max="60" data-srange="title_size"><span class="val" data-val="title_size"></span><button class="clear" data-clear="title_size" title="Reset">&#x21ba;</button></div>
  <div class="row"><label>Body size</label><input type="range" min="12" max="30" data-srange="body_size"><span class="val" data-val="body_size"></span><button class="clear" data-clear="body_size" title="Reset">&#x21ba;</button></div>
  <h3>Font family</h3>
  <div class="row"><input type="text" id="fontInput" placeholder="e.g. Georgia, Inter…"></div>
</div>
<script>
const ta = document.getElementById('src');
const statusEl = document.getElementById('status');
const fmtEl = document.getElementById('fmt');
const host = document.getElementById('host');
const errorBox = document.getElementById('error');
const posEl = document.getElementById('pos');
let known = null, ver = 0, fmt = 'html', frame = null, frameReady = false, saveTimer = null;

function setStatus(t, err) { statusEl.textContent = t; statusEl.className = err ? 'err' : ''; }
function showError(m) { errorBox.style.display = m ? 'block' : 'none'; errorBox.textContent = m || ''; }

// --- Minimal in-place DOM morph (vendored, no CDN / no deps) ---------------
// Mutates `from` to match `to`, touching only the nodes that actually differ:
// unchanged elements (and their scroll/focus/loaded-image state) are left as-is.
function imported(from, node) { return from.ownerDocument.importNode(node, true); }
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
function morph(from, to) {
  if (from.nodeName !== to.nodeName) { from.replaceWith(imported(from, to)); return; }
  if (from.nodeType === 1) morphAttrs(from, to);
  let f = from.firstChild, t = to.firstChild;
  while (t) {
    const nt = t.nextSibling;
    if (!f) { from.appendChild(imported(from, t)); t = nt; continue; }
    const nf = f.nextSibling;
    if (f.nodeType !== t.nodeType || f.nodeName !== t.nodeName) {
      from.replaceChild(imported(from, t), f);
    } else if (f.nodeType === 3 || f.nodeType === 8) {
      if (f.nodeValue !== t.nodeValue) f.nodeValue = t.nodeValue;
    } else if (f.nodeType === 1) {
      morph(f, t);
    }
    f = nf; t = nt;
  }
  while (f) { const nf = f.nextSibling; from.removeChild(f); f = nf; }
}

// --- Caret → slide (exact, via data-line) ----------------------------------
// source_line counts from the start of the markdown *body*, so first subtract
// the leading front-matter block from the caret's textarea line.
function frontMatterLines(text) {
  const lines = text.split('\n');
  if (lines[0] && lines[0].trim() === '---') {
    for (let j = 1; j < lines.length; j++) {
      if (lines[j].trim() === '---') return j + 1;
    }
  }
  return 0;
}
function caretBodyLine() {
  const before = ta.value.slice(0, ta.selectionStart);
  const line = before.split('\n').length - 1;
  return Math.max(0, line - frontMatterLines(ta.value));
}
function focusCaret(scroll) {
  if (!frame || !frameReady) return;
  const doc = frame.contentDocument;
  if (!doc) return;
  const slides = Array.from(doc.querySelectorAll('.slide'));
  if (!slides.length) return;
  const bl = caretBodyLine();
  let pick = slides[0];
  for (const s of slides) {
    if (Number(s.getAttribute('data-line') || 0) <= bl) pick = s;
    else break;
  }
  slides.forEach(s => s.classList.toggle('caret', s === pick));
  posEl.textContent = (slides.indexOf(pick) + 1) + ' / ' + slides.length;
  if (scroll) pick.scrollIntoView({ block: 'nearest' });
}

// Reverse of caretBodyLine: the byte offset of the start of a body line in the
// textarea, so clicking a slide can drop the caret onto its source.
function lineToOffset(bodyLine) {
  const target = bodyLine + frontMatterLines(ta.value);
  const lines = ta.value.split('\n');
  let pos = 0;
  for (let i = 0; i < target && i < lines.length; i++) pos += lines[i].length + 1;
  return Math.min(pos, ta.value.length);
}
// Two-way binding: click a slide in the preview to move the source caret to the
// line that produced it. Delegated on the iframe document, which survives morph.
function bindFrameClicks() {
  const doc = frame.contentDocument;
  if (!doc) return;
  doc.addEventListener('click', (e) => {
    const s = e.target.closest ? e.target.closest('.slide') : null;
    if (!s) return;
    const off = lineToOffset(Number(s.getAttribute('data-line') || 0));
    ta.focus();
    ta.selectionStart = ta.selectionEnd = off;
    const lh = parseFloat(getComputedStyle(ta).lineHeight) || 20;
    const ln = ta.value.slice(0, off).split('\n').length - 1;
    ta.scrollTop = Math.max(0, ln * lh - ta.clientHeight / 2);
    focusCaret(true);
    // Click-to-select: target this slide for the AI dock.
    setActive(blockAtTaLine(frontMatterLines(ta.value) + Number(s.getAttribute('data-line') || 0)));
  });
}

function ensureFrame() {
  frame = document.createElement('iframe');
  frame.title = 'md2any live preview';
  frame.addEventListener('load', () => { frameReady = true; bindFrameClicks(); focusCaret(true); });
  host.innerHTML = '';
  host.appendChild(frame);
  frame.src = '/deck.html?v=' + ver;
}
async function applyHtml() {
  const doc = frame && frame.contentDocument;
  if (!doc || !doc.documentElement) { frameReady = false; ensureFrame(); return; }
  const html = await (await fetch('/deck.html?v=' + ver, { cache: 'no-store' })).text();
  const next = new DOMParser().parseFromString(html, 'text/html');
  morph(doc.documentElement, next.documentElement);
  focusCaret(false);
}
// Non-HTML preview formats can't be morphed per slide — reload on each version.
function applyFallback() {
  frame = null; frameReady = false;
  if (fmt === 'pdf') host.innerHTML = '<embed src="/deck.pdf?v=' + ver + '" type="application/pdf"/>';
  else if (fmt === 'svg' || fmt === 'png') host.innerHTML = '<img src="/slides/slide-001.' + fmt + '?v=' + ver + '" alt="Slide 1"/>';
  else host.innerHTML = '<iframe src="/deck.html?v=' + ver + '" title="preview"></iframe>';
}
async function loadManifest(initial) {
  const m = await (await fetch('/manifest.json', { cache: 'no-store' })).json();
  ver = m.version; known = String(m.version); fmt = m.format; fmtEl.textContent = m.format;
  showError(m.error || '');
  if (fmt === 'html') {
    if (initial || !frame) ensureFrame();
    // On a build error keep the last good deck on screen (just show the
    // banner) — a transient typo shouldn't wipe the slide you're editing.
    else if (!m.error) await applyHtml();
  } else {
    applyFallback();
  }
}
async function tick() {
  try {
    const v = (await (await fetch('/version', { cache: 'no-store' })).text()).trim();
    if (known === null) known = v;
    else if (v !== known) { known = v; await loadManifest(false); }
  } catch (e) {}
}
async function save() {
  setStatus('saving…');
  try {
    const r = await fetch('/source', { method: 'POST', headers: { 'Content-Type': 'text/plain; charset=utf-8' }, body: ta.value });
    setStatus(r.ok ? 'saved' : 'save failed', !r.ok);
  } catch (e) { setStatus('save failed', true); }
}
function scheduleSave() { clearTimeout(saveTimer); saveTimer = setTimeout(() => { saveTimer = null; save(); }, 450); }
// Flush a pending debounced save so edits aren't lost when the tab is hidden
// or closed. sendBeacon survives unload where a normal fetch would be aborted.
function flush(beacon) {
  if (saveTimer === null) return;
  clearTimeout(saveTimer); saveTimer = null;
  if (beacon && navigator.sendBeacon) navigator.sendBeacon('/source', new Blob([ta.value], { type: 'text/plain; charset=utf-8' }));
  else save();
}
ta.addEventListener('input', () => { setStatus('editing…'); focusCaret(true); scheduleSave(); });
ta.addEventListener('keyup', () => focusCaret(true));
ta.addEventListener('click', () => focusCaret(true));
ta.addEventListener('keydown', (e) => {
  if (e.key === 'Tab') {
    e.preventDefault();
    const s = ta.selectionStart, en = ta.selectionEnd;
    ta.value = ta.value.slice(0, s) + '  ' + ta.value.slice(en);
    ta.selectionStart = ta.selectionEnd = s + 2;
  }
});
window.addEventListener('beforeunload', () => flush(true));
document.addEventListener('visibilitychange', () => { if (document.visibilityState === 'hidden') flush(true); });

// --- Style panel: reads & writes the document's front-matter -----------------
// `style:` is an inline ThemeOverride (colours/sizes/fonts); theme/aspect/etc.
// are top-level keys. The panel rewrites these in place and lets the normal
// save → rebuild → morph path apply them, so choices persist in the file.
const MANAGED_TOP = ['theme', 'aspect', 'transition', 'font'];
const THEMES = [['light','#2563eb'],['dark','#3b82f6'],['corporate','#1e3a8a'],['sepia','#a16207'],['contrast','#111111'],['midnight','#6366f1'],['terminal','#22c55e'],['pastel','#f472b6']];
let style = { style: {} };

function splitDoc() {
  const lines = ta.value.split('\n');
  if (lines[0] && lines[0].trim() === '---') {
    for (let i = 1; i < lines.length; i++) {
      if (lines[i].trim() === '---') return { pre: lines.slice(1, i), bodyFrom: i + 1, lines };
    }
  }
  return { pre: [], bodyFrom: 0, lines };
}
function unquote(s) { return s.trim().replace(/^["']|["']$/g, ''); }
function readStyle() {
  const d = splitDoc(); const st = { style: {} }; let inStyle = false;
  for (const l of d.pre) {
    if (/^style:\s*$/.test(l)) { inStyle = true; continue; }
    if (inStyle) {
      const m = l.match(/^\s+([A-Za-z_]+):\s*(.*)$/);
      if (m) { st.style[m[1]] = unquote(m[2]); continue; }
      inStyle = false;
    }
    const m = l.match(/^([A-Za-z_]+):\s*(.*)$/);
    if (m && MANAGED_TOP.includes(m[1])) st[m[1]] = unquote(m[2]);
  }
  style = st;
}
function writeStyle() {
  const d = splitDoc(); const keep = []; let inStyle = false;
  for (const l of d.pre) {
    if (/^style:\s*$/.test(l)) { inStyle = true; continue; }
    if (inStyle) { if (/^\s+\S/.test(l)) continue; inStyle = false; }
    const m = l.match(/^([A-Za-z_]+):/);
    if (m && MANAGED_TOP.includes(m[1])) continue;
    keep.push(l);
  }
  for (const k of MANAGED_TOP) if (style[k]) keep.push(k + ': ' + style[k]);
  const sk = Object.keys(style.style).filter(k => style.style[k] !== '' && style.style[k] != null);
  if (sk.length) {
    keep.push('style:');
    for (const k of sk) {
      const v = style.style[k];
      const q = typeof v === 'string' && /[#:]/.test(v);
      keep.push('  ' + k + ': ' + (q ? '"' + v + '"' : v));
    }
  }
  const body = d.lines.slice(d.bodyFrom);
  const head = (keep.length || body.length) ? ['---', ...keep, '---'] : [];
  const sel = ta.selectionStart;
  ta.value = head.concat(body).join('\n');
  ta.selectionStart = ta.selectionEnd = Math.min(sel, ta.value.length);
  setStatus('editing…'); scheduleSave(); focusCaret(true);
}
function syncControls() {
  document.querySelectorAll('#themes .swatch').forEach(s => s.classList.toggle('sel', s.dataset.theme === (style.theme || 'light')));
  document.querySelectorAll('#aspect button').forEach(b => b.classList.toggle('sel', b.dataset.v === (style.aspect || '16:9')));
  document.querySelectorAll('#transition button').forEach(b => b.classList.toggle('sel', b.dataset.v === (style.transition || 'none')));
  document.querySelectorAll('input[data-skey]').forEach(inp => { const v = style.style[inp.dataset.skey]; if (v) inp.value = v; });
  document.querySelectorAll('input[data-srange]').forEach(inp => {
    const k = inp.dataset.srange, has = style.style[k] != null && style.style[k] !== '';
    inp.value = has ? style.style[k] : inp.min;
    const val = document.querySelector('[data-val="' + k + '"]'); if (val) val.textContent = has ? style.style[k] : '–';
  });
  const fi = document.getElementById('fontInput'); if (fi) fi.value = style.font || '';
}
function buildPanel() {
  const th = document.getElementById('themes');
  THEMES.forEach(([name, col]) => {
    const b = document.createElement('div');
    b.className = 'swatch'; b.style.background = col; b.title = name; b.dataset.theme = name;
    b.onclick = () => { style.theme = name; writeStyle(); syncControls(); };
    th.appendChild(b);
  });
  const seg = (id, opts) => {
    const el = document.getElementById(id);
    opts.forEach(o => {
      const b = document.createElement('button'); b.textContent = o; b.dataset.v = o;
      b.onclick = () => { style[id] = o; writeStyle(); syncControls(); };
      el.appendChild(b);
    });
  };
  seg('aspect', ['16:9', '4:3', '16:10']);
  seg('transition', ['none', 'fade', 'push', 'wipe', 'cover']);
  document.querySelectorAll('input[data-skey]').forEach(inp =>
    inp.addEventListener('input', () => { style.style[inp.dataset.skey] = inp.value; writeStyle(); }));
  document.querySelectorAll('input[data-srange]').forEach(inp =>
    inp.addEventListener('input', () => {
      style.style[inp.dataset.srange] = Number(inp.value);
      const val = document.querySelector('[data-val="' + inp.dataset.srange + '"]'); if (val) val.textContent = inp.value;
      writeStyle();
    }));
  document.querySelectorAll('[data-clear]').forEach(b =>
    b.addEventListener('click', () => { delete style.style[b.dataset.clear]; writeStyle(); syncControls(); }));
  const fi = document.getElementById('fontInput');
  if (fi) fi.addEventListener('input', () => { style.font = fi.value.trim(); writeStyle(); });
}
document.getElementById('styleBtn').addEventListener('click', () => {
  const p = document.getElementById('panel');
  if (!p.classList.contains('open')) { readStyle(); syncControls(); }
  p.classList.toggle('open');
});
buildPanel();

// --- Generate / Save menu ----------------------------------------------------
const genBtn = document.getElementById('genBtn');
const genMenu = document.getElementById('genMenu');
genBtn.addEventListener('click', (e) => { e.stopPropagation(); genMenu.classList.toggle('open'); });
document.addEventListener('click', () => genMenu.classList.remove('open'));
function triggerDownload(href, name) {
  const a = document.createElement('a');
  a.href = href; if (name) a.download = name;
  document.body.appendChild(a); a.click(); a.remove();
}
function saveMd() {
  const url = URL.createObjectURL(new Blob([ta.value], { type: 'text/markdown' }));
  triggerDownload(url, 'deck.md');
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}
async function exportAs(fmt) {
  // Flush any pending edit to disk first so the export reflects the latest text.
  clearTimeout(saveTimer); saveTimer = null;
  setStatus('generating ' + fmt + '…');
  try {
    await save();
    triggerDownload('/export?format=' + encodeURIComponent(fmt), '');
    setStatus('generated ' + fmt);
  } catch (e) { setStatus('generate failed', true); }
}
genMenu.querySelectorAll('[data-fmt]').forEach(b =>
  b.addEventListener('click', () => { genMenu.classList.remove('open'); exportAs(b.dataset.fmt); }));
genMenu.querySelector('[data-md]').addEventListener('click', () => { genMenu.classList.remove('open'); saveMd(); });

// --- AI assistant dock -------------------------------------------------------
const aiEl = document.getElementById('ai');
const aiToggle = document.getElementById('aiToggle');
const aibar = document.getElementById('aibar');
const chatEl = document.getElementById('chat');
const aiInput = document.getElementById('aiInput');
const aiSend = document.getElementById('aiSend');
const activeChip = document.getElementById('activeChip');
const activeChipLabel = document.getElementById('activeChipLabel');
let activeSlide = null;   // 1-based logical slide the user clicked to target

// --- Slide model + surgical ops -------------------------------------------
// Parse the textarea into logical slide blocks: block 1 = title (front-matter +
// preamble), then one block per `#`/`##` heading or `---` rule (code fences are
// skipped so `#` inside code isn't mistaken for a slide start).
function parseDeck() {
  const lines = ta.value.split('\n');
  const fm = frontMatterLines(ta.value);
  const isStart = (l) => /^#{1,2}\s/.test(l) || /^---\s*$/.test(l);
  const starts = [];
  let inCode = false;
  for (let k = fm; k < lines.length; k++) {
    if (/^\s*(```|~~~)/.test(lines[k])) { inCode = !inCode; continue; }
    if (!inCode && isStart(lines[k])) starts.push(k);
  }
  const blocks = [];
  const titleEnd = starts.length ? starts[0] : lines.length;
  blocks.push({ start: 0, end: titleEnd, title: 'Title' });
  for (let s = 0; s < starts.length; s++) {
    const start = starts[s], end = (s + 1 < starts.length) ? starts[s + 1] : lines.length;
    let t = /^---\s*$/.test(lines[start]) ? '(rule)' : lines[start].replace(/^#{1,2}\s*/, '').trim();
    blocks.push({ start, end, title: t || '(untitled)' });
  }
  return { lines, blocks };
}
function slideManifest() { return parseDeck().blocks.map((b, i) => ({ n: i + 1, title: b.title })); }
function blockAtTaLine(taLine) {
  const { blocks } = parseDeck();
  let idx = 0;
  for (let i = 0; i < blocks.length; i++) if (taLine >= blocks[i].start) idx = i;
  return idx + 1;
}
function setActive(n) {
  activeSlide = n;
  const m = slideManifest()[n - 1];
  activeChip.style.display = 'flex';
  activeChipLabel.textContent = 'slide ' + n + (m && m.title ? ' · ' + m.title : '');
}
function clearActive() { activeSlide = null; activeChip.style.display = 'none'; }
// Pull surgical op blocks: ````md2any op=replace slide=12  …  ````
function extractOps(reply) {
  const ops = [];
  // Accept 3- OR 4-backtick fences: models (Grok especially) default to a
  // plain ``` fence even when told to use ````. The closing fence must match
  // the opening run length (\1), so a 4-backtick wrapper still survives inner
  // ``` code fences; a 3-backtick wrapper works for slides without code (the
  // common case — e.g. layout/valign tweaks).
  const re = /(`{3,4})[ \t]*md2any[ \t]+op=([a-z-]+)(?:[ \t]+slide=(\d+))?[^\n]*\r?\n([\s\S]*?)\1/g;
  let m;
  while ((m = re.exec(reply)) !== null) {
    ops.push({ op: m[2], n: m[3] ? Number(m[3]) : null, content: m[4].replace(/\s+$/, '') });
  }
  return ops;
}
function applyOps(ops) {
  const all = ops.find(o => o.op === 'replace-all');
  if (all) { applyDoc(all.content); return; }
  const { lines, blocks } = parseDeck();
  const actions = [];
  for (const o of ops) {
    if (o.n == null) continue;
    const b = blocks[o.n - 1];
    if (!b) continue;
    if (o.op === 'replace') actions.push({ start: b.start, end: b.end, text: o.content });
    else if (o.op === 'delete') actions.push({ start: b.start, end: b.end, text: null });
    else if (o.op === 'insert-after') actions.push({ start: b.end, end: b.end, text: o.content });
    else if (o.op === 'insert-before') actions.push({ start: b.start, end: b.start, text: o.content });
  }
  actions.sort((a, b) => b.start - a.start); // bottom-up so line indices stay valid
  for (const a of actions) {
    const repl = a.text == null ? [] : a.text.split('\n').concat(['']);
    lines.splice(a.start, a.end - a.start, ...repl);
  }
  ta.value = lines.join('\n').replace(/\n{3,}/g, '\n\n');
  ta.selectionStart = ta.selectionEnd = 0;
  setStatus('applying…'); scheduleSave(); readStyle(); syncControls(); focusCaret(true);
}
let chatHistory = [];   // [{role:'user'|'assistant', content}]
let chatBusy = false;

function toggleAi() {
  aiEl.classList.toggle('collapsed');
  const open = !aiEl.classList.contains('collapsed');
  aiToggle.innerHTML = open ? '&#9660;' : '&#9650;';
  if (open) aiInput.focus();
}
aibar.addEventListener('click', (e) => { if (e.target === aiToggle) return; toggleAi(); });
aiToggle.addEventListener('click', (e) => { e.stopPropagation(); toggleAi(); });

function addMsg(cls, text) {
  const d = document.createElement('div');
  d.className = 'msg ' + cls; d.textContent = text;
  chatEl.appendChild(d); chatEl.scrollTop = chatEl.scrollHeight;
  return d;
}
// Pull the updated-document block out of a reply, if any. The document itself
// can contain ``` code fences, so prefer an unambiguous outer fence (four
// backticks or ~~~); fall back to a *greedy* triple-backtick match that runs to
// the LAST fence, so inner code blocks don't terminate it early.
function extractDoc(reply) {
  const tag = '(?:md2any|markdown|md)?[ \\t]*\\r?\\n';
  let m = reply.match(new RegExp('````' + tag + '([\\s\\S]*?)````'))
       || reply.match(new RegExp('~~~' + tag + '([\\s\\S]*?)~~~'))
       || reply.match(new RegExp('```' + tag + '([\\s\\S]*)```')); // greedy → last ```
  if (!m) return null;
  const explain = reply.slice(0, m.index).trim();
  return { explain: explain || 'Here is the updated document.', doc: m[1].replace(/\s+$/, '') };
}
function applyDoc(doc) {
  ta.value = doc;
  ta.selectionStart = ta.selectionEnd = 0;
  setStatus('applying…'); scheduleSave(); readStyle(); syncControls(); focusCaret(true);
}
// Turn a finished reply into a clean summary + an Apply button. Prefers
// surgical op blocks; falls back to a whole-document block if the model sent one.
function finalizeBotMsg(bubble, reply) {
  let ops = extractOps(reply);
  let summary;
  if (ops.length) {
    const i = reply.search(/`{3,4}[ \t]*md2any/);
    summary = (i > 0 ? reply.slice(0, i).trim() : '') || ('Proposed ' + ops.length + ' edit' + (ops.length > 1 ? 's' : '') + '.');
  } else {
    const ext = extractDoc(reply);
    if (!ext) { bubble.textContent = reply; return; }   // plain answer
    ops = [{ op: 'replace-all', n: null, content: ext.doc }];
    summary = ext.explain;
  }
  bubble.textContent = summary;
  const wrap = document.createElement('div'); wrap.className = 'apply';
  const btn = document.createElement('button');
  const label = ops.length === 1 && ops[0].op === 'replace-all'
    ? '✓ Apply' : '✓ Apply ' + ops.length + ' edit' + (ops.length > 1 ? 's' : '');
  btn.textContent = label;
  btn.onclick = () => { applyOps(ops); btn.textContent = '✓ Applied'; btn.disabled = true; clearActive(); };
  wrap.appendChild(btn); bubble.appendChild(wrap);
  chatEl.scrollTop = chatEl.scrollHeight;
}
async function sendChat() {
  const text = aiInput.value.trim();
  if (!text || chatBusy) return;
  aiInput.value = ''; aiInput.style.height = 'auto';
  addMsg('user', text);
  chatHistory.push({ role: 'user', content: text });
  chatBusy = true; aiSend.disabled = true;
  const bubble = addMsg('bot busy', 'thinking…');
  let acc = '', errored = null;
  try {
    const r = await fetch('/chat', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ messages: chatHistory, doc: ta.value, slides: slideManifest(), active: activeSlide }) });
    const reader = r.body.getReader();
    const dec = new TextDecoder();
    let buf = '';
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      buf += dec.decode(value, { stream: true });
      let nl;
      while ((nl = buf.indexOf('\n')) >= 0) {
        const line = buf.slice(0, nl).trim(); buf = buf.slice(nl + 1);
        if (!line) continue;
        let obj; try { obj = JSON.parse(line); } catch (e) { continue; }
        if (obj.delta != null) {
          if (!acc) bubble.classList.remove('busy');
          acc += obj.delta; bubble.textContent = acc;
          chatEl.scrollTop = chatEl.scrollHeight;
        } else if (obj.error) {
          errored = obj.error;
        }
      }
    }
  } catch (e) {
    errored = 'request failed: ' + e.message;
  }
  if (errored) {
    bubble.remove(); addMsg('err', errored);
  } else {
    chatHistory.push({ role: 'assistant', content: acc });
    bubble.classList.remove('busy');
    finalizeBotMsg(bubble, acc);
  }
  chatBusy = false; aiSend.disabled = false; aiInput.focus();
}
aiSend.addEventListener('click', sendChat);
aiInput.addEventListener('keydown', (e) => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); sendChat(); } });
document.querySelectorAll('#aichips .chip').forEach(c =>
  c.addEventListener('click', () => { if (chatBusy) return; aiInput.value = c.textContent; sendChat(); }));
aiInput.addEventListener('input', () => { aiInput.style.height = 'auto'; aiInput.style.height = Math.min(aiInput.scrollHeight, 120) + 'px'; });
document.getElementById('activeChipX').addEventListener('click', clearActive);

fetch('/source', { cache: 'no-store' }).then(r => r.text()).then(t => { ta.value = t; focusCaret(true); }).catch(() => {});
setInterval(tick, 500);
loadManifest(true).catch(() => showError('unable to load preview manifest'));
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
            export: Arc::new(|_: &str| -> Result<ExportFile, String> { Err("n/a".into()) }),
            chat: Arc::new(
                |_: &str, _: &mut dyn FnMut(&str) -> Result<(), String>| -> Result<(), String> {
                    Err("n/a".into())
                },
            ),
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
            export: Arc::new(|_: &str| -> Result<ExportFile, String> { Err("n/a".into()) }),
            chat: Arc::new(
                |_: &str, _: &mut dyn FnMut(&str) -> Result<(), String>| -> Result<(), String> {
                    Err("n/a".into())
                },
            ),
        };

        let json = manifest_json(&state);
        assert!(json.contains("\"format\":\"html\""));
        assert!(json.contains("bad \\\"quote\\\"\\nnext"), "{json}");
    }
}
