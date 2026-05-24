//! Minimal preview server for `--serve`. No deps beyond std.
//!
//! - Builds the deck once to PDF on startup.
//! - Spawns a watcher thread that re-renders when any input file's mtime
//!   changes (same model as `--watch`).
//! - Serves three routes on a localhost-only listener:
//!     - `GET /`          → HTML wrapper embedding the PDF, polls /version
//!     - `GET /deck.pdf`  → current PDF bytes
//!     - `GET /version`   → integer incremented on every rebuild
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
}

impl Default for ServeOpts {
    fn default() -> Self {
        ServeOpts {
            port: 8421,
            bind: "127.0.0.1".into(),
        }
    }
}

struct State {
    pdf: Vec<u8>,
    version: u64,
    error: Option<String>,
}

pub fn run<F>(opts: ServeOpts, paths: Vec<PathBuf>, mut build: F) -> std::io::Result<()>
where
    F: FnMut() -> Result<Vec<u8>, String> + Send + 'static,
{
    let initial = match build() {
        Ok(bytes) => State {
            pdf: bytes,
            version: 1,
            error: None,
        },
        Err(e) => {
            eprintln!("md2any: initial build failed: {e}");
            State {
                pdf: error_pdf(&e),
                version: 1,
                error: Some(e),
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
                    Ok(bytes) => {
                        let mut s = watcher_state.lock().unwrap();
                        s.pdf = bytes;
                        s.version += 1;
                        s.error = None;
                        eprintln!("md2any: rebuilt (v{})", s.version);
                    }
                    Err(e) => {
                        let mut s = watcher_state.lock().unwrap();
                        s.error = Some(e.clone());
                        s.version += 1;
                        s.pdf = error_pdf(&e);
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
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut buf = [0u8; 2048];
    let n = stream.read(&mut buf)?;
    let req = String::from_utf8_lossy(&buf[..n]);
    let path = req
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");
    let path = path.split('?').next().unwrap_or("/");

    match path {
        "/" => {
            let html = INDEX_HTML;
            write_response(
                &mut stream,
                200,
                "OK",
                "text/html; charset=utf-8",
                html.as_bytes(),
            )?;
        }
        "/version" => {
            let v = state.lock().unwrap().version;
            let body = v.to_string();
            write_response(&mut stream, 200, "OK", "text/plain", body.as_bytes())?;
        }
        "/deck.pdf" => {
            let s = state.lock().unwrap();
            let bytes = s.pdf.clone();
            drop(s);
            write_response(&mut stream, 200, "OK", "application/pdf", &bytes)?;
        }
        _ => {
            write_response(&mut stream, 404, "Not Found", "text/plain", b"not found")?;
        }
    }
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
  embed { width: 100%; height: 100%; border: 0; }
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
<embed id="pdf" src="/deck.pdf" type="application/pdf"/>
<div id="toast">reloading…</div>
<script>
let known = null;
const toast = document.getElementById('toast');
async function tick() {
  try {
    const r = await fetch('/version', { cache: 'no-store' });
    const v = (await r.text()).trim();
    if (known === null) {
      known = v;
    } else if (v !== known) {
      known = v;
      toast.classList.add('show');
      // Bump src with a cache buster so the embed actually refetches.
      document.getElementById('pdf').src = '/deck.pdf?v=' + v;
      setTimeout(() => toast.classList.remove('show'), 800);
    }
  } catch (e) {}
}
setInterval(tick, 500);
tick();
</script>
</body>
</html>
"#;

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
