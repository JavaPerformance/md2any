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
        // Image search across free + keyed providers. `GET /image-search?q=…`
        // → JSON array of {title,image_url,thumb_url,license,author,source}.
        // Backs the AI dock's photo-finding and any manual insert UI.
        ("GET", "/image-search") => {
            let q = query_param(&raw_path, "q").unwrap_or_default();
            let hits = crate::imgsearch::search(&q, 4);
            let body = serde_json::to_vec(&hits).unwrap_or_else(|_| b"[]".to_vec());
            write_response(&mut stream, 200, "OK", "application/json", &body)?;
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
        // Editor: download remote images in the posted doc into assets/ next to
        // the source file and rewrite the links, returning {doc, count}. Makes
        // the deck self-contained; the dock calls this after the AI inserts a
        // remote image URL.
        ("POST", "/localize") => {
            let doc = String::from_utf8_lossy(&body).into_owned();
            let src = state.lock().unwrap().edit_source.clone();
            match src.as_deref().and_then(|p| p.parent()) {
                Some(base) => match crate::image::localize_doc(&doc, base) {
                    Ok((new_doc, count)) => {
                        let resp =
                            serde_json::json!({ "doc": new_doc, "count": count }).to_string();
                        write_response(
                            &mut stream,
                            200,
                            "OK",
                            "application/json",
                            resp.as_bytes(),
                        )?;
                    }
                    Err(e) => write_response(
                        &mut stream,
                        500,
                        "Error",
                        "text/plain",
                        e.to_string().as_bytes(),
                    )?,
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
        (k == key).then(|| url_decode(v))
    })
}

/// Decode an `application/x-www-form-urlencoded` value: `+` → space and
/// `%XX` → byte. Without this, multi-word query params arrive as one token.
fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                if let (Some(h), Some(l)) = (hi, lo) {
                    out.push((h * 16 + l) as u8);
                    i += 3;
                    continue;
                }
                out.push(b'%');
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
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
  :root {
    --bg: #0a0f1f; --bg2: #0e1426; --bg3: #161f38; --bg3h: #1e2945;
    --line: rgba(148,163,184,.13); --line2: rgba(148,163,184,.22);
    --text: #e8edf8; --muted: #93a0bd; --faint: #66718f;
    --accent: #818cf8; --accent2: #c084fc; --ok: #34d399;
    --grad: linear-gradient(135deg, #6366f1, #a855f7);
    --shadow: 0 10px 30px rgba(0,0,0,.45);
  }
  html, body { margin: 0; height: 100%; font-family: system-ui, -apple-system, "Segoe UI", sans-serif; -webkit-font-smoothing: antialiased; }
  body { display: flex; flex-direction: column; background: var(--bg); color: var(--text); }
  * { scrollbar-width: thin; scrollbar-color: var(--line2) transparent; }
  ::-webkit-scrollbar { width: 10px; height: 10px; }
  ::-webkit-scrollbar-thumb { background: var(--line2); border-radius: 6px; border: 2px solid transparent; background-clip: content-box; }
  ::-webkit-scrollbar-thumb:hover { background: var(--faint); background-clip: content-box; }
  button { font-family: inherit; transition: background .15s, color .15s, border-color .15s, transform .08s, box-shadow .15s; }
  button:active { transform: translateY(1px); }
  #bar {
    display: flex; align-items: center; gap: 13px; padding: 9px 14px;
    background: linear-gradient(180deg, #11192f, #0c1322);
    border-bottom: 1px solid var(--line); font-size: 13px; flex: 0 0 auto;
    box-shadow: 0 1px 0 rgba(0,0,0,.4), 0 6px 18px rgba(0,0,0,.18); z-index: 5;
  }
  #bar b { font-weight: 700; background: var(--grad); -webkit-background-clip: text; background-clip: text; color: transparent; letter-spacing: .2px; }
  #bar .dot { color: var(--faint); }
  #status { display: inline-flex; align-items: center; gap: 6px; color: var(--muted); font-size: 12px; padding: 3px 9px; border-radius: 999px; background: rgba(148,163,184,.08); border: 1px solid var(--line); }
  #status::before { content: ""; width: 6px; height: 6px; border-radius: 50%; background: var(--ok); box-shadow: 0 0 6px var(--ok); }
  #status.err { color: #fca5a5; }
  #status.err::before { background: #f87171; box-shadow: 0 0 6px #f87171; }
  #pos { color: var(--faint); font-variant-numeric: tabular-nums; }
  #fmt { margin-left: auto; color: var(--muted); text-transform: uppercase; letter-spacing: .08em; font-size: 11px; }
  #main { flex: 1 1 auto; display: flex; min-height: 0; }
  #edit { width: 44%; min-width: 220px; display: flex; }
  #src {
    width: 100%; border: 0; resize: none; outline: none;
    background: var(--bg); color: #d7def0; padding: 16px 18px; caret-color: var(--accent);
    font: 13px/1.6 ui-monospace, "DejaVu Sans Mono", monospace;
    tab-size: 2; white-space: pre; overflow: auto;
  }
  #src::selection { background: rgba(129,140,248,.32); }
  #divider { width: 1px; background: var(--line); box-shadow: 1px 0 0 rgba(0,0,0,.3); }
  /* Live-DOM preview: the iframe holds the rendered deck and we morph each
     rebuild into its existing DOM, so only the changed slide's nodes update —
     scroll position and image-load state on every other slide are preserved. */
  #view { flex: 1 1 auto; display: flex; flex-direction: column; min-width: 0; background: radial-gradient(120% 80% at 50% -10%, #18233f 0%, #0a0f1f 60%); }
  #error {
    display: none; margin: 10px;
    padding: 11px 13px; background: linear-gradient(180deg,#7f1d1d,#641818); color: #fff; font-size: 12px;
    border-radius: 8px; z-index: 4; white-space: pre-wrap; flex: 0 0 auto; box-shadow: var(--shadow); border: 1px solid rgba(248,113,113,.4);
  }
  #host { flex: 1 1 auto; min-height: 0; overflow: auto; padding: 4px; }
  #host iframe, #host embed { width: 100%; height: 100%; border: 0; background: transparent; display: block; }
  #host img { width: 100%; display: block; background: #fff; border-radius: 8px; }
  #bar button.tool { background: rgba(148,163,184,.1); color: var(--text); border: 1px solid var(--line); border-radius: 7px; padding: 5px 11px; cursor: pointer; font-size: 12px; font-weight: 500; }
  #bar button.tool:hover { background: var(--bg3h); border-color: var(--line2); }
  #menu { position: relative; }
  #genMenu { display: none; position: absolute; right: 0; top: 34px; background: var(--bg2); border: 1px solid var(--line2); border-radius: 10px; box-shadow: var(--shadow); min-width: 176px; z-index: 30; padding: 5px; }
  #genMenu.open { display: block; animation: pop .14s ease; }
  @keyframes pop { from { opacity: 0; transform: translateY(-6px) scale(.98); } to { opacity: 1; transform: none; } }
  #genMenu button { display: block; width: 100%; text-align: left; background: none; border: 0; color: var(--muted); padding: 7px 11px; font-size: 12px; cursor: pointer; border-radius: 6px; }
  #genMenu button:hover { background: var(--bg3); color: var(--text); }
  #genMenu .sep { height: 1px; background: var(--line); margin: 5px 4px; }
  /* Slide-in style panel: front-matter-backed theme/colour/size controls. */
  #panel { position: fixed; top: 0; right: 0; height: 100%; width: 290px; box-sizing: border-box;
    background: linear-gradient(180deg,#0e1426,#0b1020); border-left: 1px solid var(--line2); box-shadow: -16px 0 40px rgba(0,0,0,.5);
    transform: translateX(100%); transition: transform .22s cubic-bezier(.2,.8,.2,1); z-index: 20; overflow: auto; padding: 16px; }
  #panel.open { transform: none; }
  #panel h3 { margin: 18px 0 7px; font-size: 11px; text-transform: uppercase; letter-spacing: .1em; color: var(--faint); }
  #panel h3:first-child { margin-top: 0; }
  #panel .row { display: flex; align-items: center; gap: 8px; margin: 7px 0; font-size: 12px; color: var(--text); }
  #panel .row label { flex: 1 1 auto; }
  #panel .swatches { display: flex; flex-wrap: wrap; gap: 7px; }
  #panel .swatch { width: 26px; height: 26px; border-radius: 7px; border: 2px solid transparent; cursor: pointer; box-shadow: 0 2px 6px rgba(0,0,0,.35); transition: transform .12s; }
  #panel .swatch:hover { transform: scale(1.12); }
  #panel .swatch.sel { border-color: #fff; box-shadow: 0 0 0 2px var(--accent); }
  #panel .seg { display: flex; flex-wrap: wrap; gap: 5px; }
  #panel .seg button { background: var(--bg3); color: var(--muted); border: 1px solid var(--line); border-radius: 7px; padding: 4px 9px; font-size: 12px; cursor: pointer; }
  #panel .seg button:hover { color: var(--text); border-color: var(--line2); }
  #panel .seg button.sel { background: var(--grad); color: #fff; border-color: transparent; }
  #panel input[type=range] { flex: 1 1 auto; min-width: 0; accent-color: var(--accent); }
  #panel input[type=color] { width: 34px; height: 24px; border: 0; background: none; padding: 0; cursor: pointer; }
  #panel .val { width: 30px; text-align: right; color: var(--muted); font-variant-numeric: tabular-nums; }
  #panel .clear { background: none; border: 0; color: var(--faint); cursor: pointer; font-size: 13px; padding: 0 2px; }
  #panel .clear:hover { color: var(--text); }
  #panel input[type=text] { width: 100%; background: var(--bg); color: var(--text); border: 1px solid var(--line); border-radius: 7px; padding: 6px 8px; font-size: 12px; box-sizing: border-box; }
  #panel input[type=text]:focus { outline: none; border-color: var(--accent); }
  /* AI assistant dock (bottom). */
  #ai { flex: 0 0 auto; display: flex; flex-direction: column; border-top: 1px solid var(--line2); background: linear-gradient(180deg,#0e1426,#0a0f1f); max-height: 44%; box-shadow: 0 -12px 30px rgba(0,0,0,.3); }
  #ai.collapsed { max-height: none; }
  #ai.collapsed #chat, #ai.collapsed #airow { display: none; }
  #aibar { display: flex; align-items: center; gap: 10px; padding: 9px 14px; font-size: 13px; color: var(--text); cursor: pointer; flex: 0 0 auto; font-weight: 600; }
  #aibar #aihint { color: var(--faint); font-size: 12px; font-weight: 400; }
  #aibar button { margin-left: auto; }
  #chat { flex: 1 1 auto; overflow: auto; padding: 14px; display: flex; flex-direction: column; gap: 10px; min-height: 140px; }
  .msg { max-width: 82%; padding: 9px 12px; border-radius: 13px; font-size: 13px; line-height: 1.48; white-space: pre-wrap; word-break: break-word; box-shadow: 0 2px 8px rgba(0,0,0,.22); animation: rise .18s ease; }
  @keyframes rise { from { opacity: 0; transform: translateY(5px); } to { opacity: 1; transform: none; } }
  .msg.user { align-self: flex-end; background: var(--grad); color: #fff; border-bottom-right-radius: 4px; }
  .msg.bot { align-self: flex-start; background: var(--bg3); color: var(--text); border: 1px solid var(--line); border-bottom-left-radius: 4px; }
  .msg.err { align-self: flex-start; background: #7f1d1d; color: #fff; }
  .msg.busy { opacity: .6; font-style: italic; }
  .msg .apply { margin-top: 9px; }
  .msg .apply button { background: linear-gradient(135deg,#16a34a,#22c55e); color: #fff; border: 0; border-radius: 7px; padding: 7px 13px; font-size: 12px; font-weight: 600; cursor: pointer; box-shadow: 0 2px 10px rgba(34,197,94,.35); }
  .msg .apply button:hover { filter: brightness(1.08); }
  .msg .apply button:disabled { background: var(--bg3h); box-shadow: none; cursor: default; filter: none; }
  #aichips { display: flex; flex-wrap: wrap; gap: 7px; padding: 0 14px 10px; flex: 0 0 auto; }
  #ai.collapsed #aichips { display: none; }
  .chip { background: rgba(148,163,184,.08); color: var(--muted); border: 1px solid var(--line); border-radius: 999px; padding: 5px 12px; font-size: 12px; cursor: pointer; }
  .chip:hover { background: var(--bg3h); color: var(--text); border-color: var(--accent); transform: translateY(-1px); }
  #imgFindChip { border-color: var(--line2); color: var(--accent); }
  #activeChip { display: none; align-items: center; gap: 8px; margin: 0 14px 8px; padding: 5px 11px; background: rgba(129,140,248,.12); border: 1px solid rgba(129,140,248,.4); border-radius: 8px; font-size: 12px; color: #c7cdff; }
  #activeChip b { color: #fff; }
  #activeChip button { margin-left: auto; background: none; border: 0; color: var(--accent); cursor: pointer; font-size: 13px; }
  #airow { display: flex; gap: 8px; padding: 10px 14px; border-top: 1px solid var(--line); flex: 0 0 auto; }
  #aiInput { flex: 1 1 auto; resize: none; background: var(--bg); color: var(--text); border: 1px solid var(--line); border-radius: 9px; padding: 9px 11px; font: 13px/1.4 system-ui, sans-serif; }
  #aiInput:focus { outline: none; border-color: var(--accent); box-shadow: 0 0 0 3px rgba(129,140,248,.18); }
  #aiSend { background: var(--grad); color: #fff; border: 0; border-radius: 9px; padding: 0 16px; font-size: 13px; font-weight: 600; cursor: pointer; box-shadow: 0 2px 12px rgba(99,102,241,.35); }
  #aiSend:hover { filter: brightness(1.08); }
  #imgsearch { padding: 8px 14px; background: rgba(0,0,0,.18); border-top: 1px solid var(--line); }
  #imgrow { display: flex; gap: 8px; }
  #imgq { flex: 1 1 auto; background: var(--bg); color: var(--text); border: 1px solid var(--line); border-radius: 9px; padding: 7px 10px; font: 13px system-ui, sans-serif; }
  #imgq:focus { outline: none; border-color: var(--accent); box-shadow: 0 0 0 3px rgba(129,140,248,.18); }
  #imgresults { display: flex; flex-wrap: wrap; gap: 9px; margin-top: 9px; max-height: 220px; overflow: auto; }
  #imgresults .hit { width: 120px; cursor: pointer; border: 1px solid var(--line); border-radius: 9px; overflow: hidden; background: var(--bg3); transition: transform .12s, border-color .15s, box-shadow .15s; }
  #imgresults .hit img { width: 120px; height: 80px; object-fit: cover; display: block; }
  #imgresults .hit .cap { font-size: 10px; color: var(--muted); padding: 4px 6px; line-height: 1.25; }
  #imgresults .hit:hover { border-color: var(--accent); transform: translateY(-2px); box-shadow: 0 8px 18px rgba(0,0,0,.4); }
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
    <button class="chip" id="imgFindChip">🖼 Find an image…</button>
  </div>
  <div id="imgsearch" hidden>
    <div id="imgrow">
      <input id="imgq" type="text" placeholder="search photos — e.g. “Zilog Z80 chip”" />
      <button class="tool" id="imggo">Search</button>
    </div>
    <div id="imgresults"></div>
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
// Count substantive lines (prose/images/lists/headings); blanks and <!-- -->
// directive comments don't count, so adding a directive isn't seen as growth
// and dropping a paragraph IS seen as loss.
function substantiveLines(arr) {
  return arr.filter(l => { const t = l.trim(); return t && !t.startsWith('<!--'); }).length;
}
function applyOps(ops) {
  const all = ops.find(o => o.op === 'replace-all');
  if (all) {
    // replace-all rewrites the WHOLE deck, so a model omission loses content
    // across every slide. Guard it the same way as per-slide replaces.
    const oldN = substantiveLines(ta.value.split('\n'));
    const newN = substantiveLines(all.content.split('\n'));
    if (newN < oldN && !confirm(
          'This rewrites the whole deck and removes ' + (oldN - newN)
          + ' line(s) of existing content.\n\nApply anyway?')) {
      return false;
    }
    applyDoc(all.content);
    return true;
  }
  const { lines, blocks } = parseDeck();
  const actions = [];
  let lost = 0; // substantive lines a replace would drop (model content-loss guard)
  for (const o of ops) {
    if (o.n == null) continue;
    const b = blocks[o.n - 1];
    if (!b) continue;
    if (o.op === 'replace') {
      const oldN = substantiveLines(lines.slice(b.start, b.end));
      const newN = substantiveLines(o.content.split('\n'));
      if (newN < oldN) lost += oldN - newN;
      actions.push({ start: b.start, end: b.end, text: o.content });
    }
    else if (o.op === 'delete') actions.push({ start: b.start, end: b.end, text: null });
    else if (o.op === 'insert-after') actions.push({ start: b.end, end: b.end, text: o.content });
    else if (o.op === 'insert-before') actions.push({ start: b.start, end: b.start, text: o.content });
  }
  // Models occasionally regenerate a slide for op=replace and omit existing
  // prose. If the edit drops content the user didn't ask to remove, confirm.
  if (lost > 0 && !confirm(
        'This edit removes ' + lost + ' line(s) of existing content (paragraphs, '
        + 'images, etc.) that you may not have asked to delete.\n\nApply anyway?')) {
    return false;
  }
  actions.sort((a, b) => b.start - a.start); // bottom-up so line indices stay valid
  for (const a of actions) {
    const repl = a.text == null ? [] : a.text.split('\n').concat(['']);
    lines.splice(a.start, a.end - a.start, ...repl);
  }
  ta.value = lines.join('\n').replace(/\n{3,}/g, '\n\n');
  ta.selectionStart = ta.selectionEnd = 0;
  setStatus('applying…'); scheduleSave(); readStyle(); syncControls(); focusCaret(true);
  // If the AI inserted remote image URLs, download them into assets/ so the
  // deck is self-contained (and the preview doesn't re-fetch every rebuild).
  if (ops.some(o => o.content && /\]\(https?:\/\//.test(o.content))) localizeRemoteImages();
  return true;
}
// Download remote (http) markdown images into assets/ and rewrite the links,
// so the deck is self-contained. Posts the current doc to /localize and swaps
// in the rewritten version. Safe no-op if there are no remote images.
async function localizeRemoteImages() {
  if (!/\]\(https?:\/\//.test(ta.value)) return;
  setStatus('downloading images…');
  try {
    const r = await fetch('/localize', { method: 'POST', body: ta.value });
    if (!r.ok) { setStatus('image download failed'); return; }
    const { doc, count } = await r.json();
    if (count > 0 && doc && doc !== ta.value) {
      ta.value = doc;
      scheduleSave(); readStyle(); syncControls();
      setStatus('downloaded ' + count + ' image' + (count > 1 ? 's' : '') + ' to assets/');
    } else {
      setStatus('ready');
    }
  } catch (e) { setStatus('image download failed'); }
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
  btn.onclick = () => {
    if (!applyOps(ops)) return; // guard declined (e.g. would drop content)
    btn.textContent = '✓ Applied'; btn.disabled = true; clearActive();
  };
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
  c.addEventListener('click', () => {
    if (c.id === 'imgFindChip') return; // handled below — opens the image panel
    if (chatBusy) return; aiInput.value = c.textContent; sendChat();
  }));
aiInput.addEventListener('input', () => { aiInput.style.height = 'auto'; aiInput.style.height = Math.min(aiInput.scrollHeight, 120) + 'px'; });
document.getElementById('activeChipX').addEventListener('click', clearActive);

// Manual image search + insert (uses the same /image-search backend as the AI).
const imgPanel = document.getElementById('imgsearch');
const imgQ = document.getElementById('imgq');
const imgResults = document.getElementById('imgresults');
document.getElementById('imgFindChip').addEventListener('click', () => {
  imgPanel.hidden = !imgPanel.hidden;
  if (!imgPanel.hidden) imgQ.focus();
});
async function runImageSearch() {
  const q = imgQ.value.trim();
  if (!q) return;
  imgResults.textContent = 'searching…';
  try {
    const r = await fetch('/image-search?q=' + encodeURIComponent(q));
    const hits = await r.json();
    imgResults.textContent = '';
    if (!hits.length) { imgResults.textContent = 'no results'; return; }
    for (const h of hits) {
      const card = document.createElement('div');
      card.className = 'hit';
      card.title = h.title + ' — ' + h.source + ' (' + h.license + ')';
      const im = document.createElement('img');
      im.src = h.thumb_url || h.image_url; im.loading = 'lazy';
      const cap = document.createElement('div');
      cap.className = 'cap';
      cap.textContent = (h.author || h.source) + ' · ' + h.license;
      card.appendChild(im); card.appendChild(cap);
      card.addEventListener('click', () => insertImage(h));
      imgResults.appendChild(card);
    }
  } catch (e) { imgResults.textContent = 'search failed'; }
}
document.getElementById('imggo').addEventListener('click', runImageSearch);
imgQ.addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); runImageSearch(); } });
function insertImage(h) {
  const alt = (h.title || 'image').replace(/[\[\]]/g, '');
  const credit = (h.author || h.source) ? ('\n*Photo: ' + (h.author || h.source) + (h.license ? ' / ' + h.license : '') + '*') : '';
  const md = '\n![' + alt + '](' + h.image_url + ')' + credit + '\n';
  const at = ta.selectionStart || ta.value.length;
  ta.value = ta.value.slice(0, at) + md + ta.value.slice(at);
  ta.selectionStart = ta.selectionEnd = at + md.length;
  setStatus('inserting image…'); scheduleSave(); readStyle(); syncControls(); focusCaret(true);
  imgPanel.hidden = true; imgResults.textContent = ''; imgQ.value = '';
  localizeRemoteImages(); // download into assets/ + rewrite
}

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
    fn url_decode_handles_plus_and_percent() {
        assert_eq!(url_decode("a+b"), "a b");
        assert_eq!(url_decode("a%20b"), "a b");
        assert_eq!(url_decode("Zilog+Z80+chip"), "Zilog Z80 chip");
        assert_eq!(url_decode("100%25"), "100%");
        assert_eq!(url_decode("no-encoding"), "no-encoding");
    }

    #[test]
    fn query_param_decodes_value() {
        assert_eq!(
            query_param("/image-search?q=game+boy", "q").as_deref(),
            Some("game boy")
        );
    }

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
