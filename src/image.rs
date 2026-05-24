//! Image loading: local files, remote URLs, SVG rasterisation, plus a
//! visible placeholder when anything fails.
//!
//! Renderers need three things from an image: the raw bytes, the pixel
//! dimensions (for aspect-fitting on the slide), and the format
//! identifier (so the renderer can pick the right embedding strategy —
//! `/DCTDecode` for JPEG, raw IDAT inflate for PNG). For local PNG / JPEG
//! we sniff magic bytes and parse the dimension field directly — no
//! image crate dependency.
//!
//! For remote URLs (`http://`, `https://`, feature-gated on
//! `remote-images`, default on) we fetch via `ureq` with bounded retry,
//! cap payloads at 20 MB, sniff before caching to avoid poisoning the
//! cache with garbage, and write to a platform-standard cache directory
//! so subsequent renders are zero-roundtrip.
//!
//! For SVG (feature-gated on `svg`, default on) we rasterise via
//! `resvg` + `tiny-skia` using the bundled DejaVu fonts so text in SVGs
//! renders identically regardless of the build machine's installed fonts.
//!
//! On any failure — network error, 404, garbage body, cap exceeded,
//! missing local file — we substitute a visible "image failed to load"
//! placeholder rather than aborting the render, and warn on stderr so
//! the failure stays visible in CI logs.

#[cfg(feature = "remote-images")]
use anyhow::anyhow;
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
#[cfg(feature = "remote-images")]
use std::sync::OnceLock;

/// Metadata + raw bytes of a loaded image. `ext` is `"png"` or `"jpeg"`.
#[derive(Debug, Clone)]
pub struct ImageMeta {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub ext: &'static str,
}

/// Runtime options for remote image loading.
///
/// The CLI sets this once from command-line flags. Library callers that do
/// nothing get platform defaults.
#[derive(Debug, Clone)]
pub struct RemoteImageOptions {
    pub cache_enabled: bool,
    pub cache_dir: Option<PathBuf>,
    pub user_agent: Option<String>,
}

impl Default for RemoteImageOptions {
    fn default() -> Self {
        Self {
            cache_enabled: true,
            cache_dir: None,
            user_agent: None,
        }
    }
}

/// Configure remote image loading for this process.
///
/// Calling this is optional; the default uses the platform cache directory.
/// **Single-set:** backed by `OnceLock`, so the *first* call wins and any
/// subsequent call is silently ignored. The CLI calls this exactly once at
/// startup; library embedders should do the same before any render runs.
pub fn configure_remote_images(options: RemoteImageOptions) {
    set_remote_image_options(options);
}

#[cfg(feature = "remote-images")]
static REMOTE_IMAGE_OPTIONS: OnceLock<RemoteImageOptions> = OnceLock::new();

#[cfg(feature = "remote-images")]
fn set_remote_image_options(options: RemoteImageOptions) {
    let _ = REMOTE_IMAGE_OPTIONS.set(options);
}

#[cfg(not(feature = "remote-images"))]
fn set_remote_image_options(_options: RemoteImageOptions) {}

#[cfg(feature = "remote-images")]
fn remote_image_options() -> RemoteImageOptions {
    REMOTE_IMAGE_OPTIONS.get().cloned().unwrap_or_default()
}

/// Load an image from a markdown reference. Local paths go through
/// `load`; `http://` / `https://` URLs are fetched via [`fetch_remote`]
/// when the `remote-images` feature is enabled.
///
/// `base_dir` is the directory of the markdown file — relative local
/// paths resolve against it.
pub fn load_any(base_dir: &Path, src: &str) -> Result<ImageMeta> {
    if src.starts_with("http://") || src.starts_with("https://") {
        return fetch_remote(src);
    }
    let path = if Path::new(src).is_absolute() {
        std::path::PathBuf::from(src)
    } else {
        base_dir.join(src)
    };
    load(&path)
}

/// Load an image, or fall back to a visible "image failed" placeholder
/// rather than aborting the whole render. Prints a one-line warning to
/// stderr so the failure is still visible in CI logs even when the deck
/// itself renders successfully.
///
/// Used by every renderer's image-collection step. If you genuinely want
/// hard-fail behaviour on image errors, call [`load_any`] directly.
pub fn load_any_or_placeholder(base_dir: &Path, src: &str) -> ImageMeta {
    match load_any(base_dir, src) {
        Ok(meta) => meta,
        Err(e) => {
            let reason = format!("{:#}", e);
            eprintln!("md2any: warning: image failed, substituting placeholder");
            eprintln!("  src:    {}", src);
            eprintln!("  reason: {}", reason);
            placeholder_meta(src, &reason)
        }
    }
}

/// Build a visible "image failed to load" placeholder. Tries the SVG
/// pipeline first (so the URL and error appear inside the image); falls
/// back to a tiny solid-colour PNG if the `svg` feature is off, so the
/// caller always gets a usable [`ImageMeta`].
pub fn placeholder_meta(src: &str, reason: &str) -> ImageMeta {
    #[cfg(feature = "svg")]
    {
        if let Ok(meta) = svg_placeholder(src, reason) {
            return meta;
        }
    }
    static_placeholder_meta()
}

#[cfg(feature = "svg")]
fn svg_placeholder(src: &str, reason: &str) -> Result<ImageMeta> {
    let svg = format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="600" height="400" viewBox="0 0 600 400">
  <rect width="600" height="400" fill="#fef2f2"/>
  <rect x="4" y="4" width="592" height="392" fill="none" stroke="#dc2626" stroke-width="6"/>
  <line x1="80" y1="80" x2="180" y2="180" stroke="#dc2626" stroke-width="8" stroke-linecap="round"/>
  <line x1="180" y1="80" x2="80" y2="180" stroke="#dc2626" stroke-width="8" stroke-linecap="round"/>
  <text x="300" y="220" font-family="DejaVu Sans" font-size="26" font-weight="bold" text-anchor="middle" fill="#991b1b">Image failed to load</text>
  <text x="300" y="270" font-family="DejaVu Sans" font-size="14" text-anchor="middle" fill="#7f1d1d">{}</text>
  <text x="300" y="320" font-family="DejaVu Sans" font-size="12" text-anchor="middle" fill="#7f1d1d">{}</text>
</svg>"##,
        escape_xml(&truncate_for_display(src, 80)),
        escape_xml(&truncate_for_display(reason, 100)),
    );
    rasterize_svg(svg.as_bytes(), "image-failed placeholder")
}

fn truncate_for_display(s: &str, max_chars: usize) -> String {
    let collapsed: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if collapsed.chars().count() <= max_chars {
        return collapsed;
    }
    let head: String = collapsed
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect();
    format!("{}…", head)
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Last-resort placeholder: a 320×200 red-on-pink PNG with no text. Used
/// only when the `svg` feature is off so the SVG rasteriser isn't
/// available. The bytes are constructed once at first use via [`std::sync::OnceLock`].
fn static_placeholder_meta() -> ImageMeta {
    use std::sync::OnceLock;
    static BYTES: OnceLock<Vec<u8>> = OnceLock::new();
    let bytes = BYTES.get_or_init(build_static_placeholder_png).clone();
    ImageMeta {
        bytes,
        width: 320,
        height: 200,
        ext: "png",
    }
}

/// Build a minimal valid 320×200 PNG: a pink background with a red border
/// and a red X. Hand-encoded so we have a fallback even when `svg` is off.
fn build_static_placeholder_png() -> Vec<u8> {
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::Write;

    const W: u32 = 320;
    const H: u32 = 200;
    let mut raw: Vec<u8> = Vec::with_capacity(((W * 3) + 1) as usize * H as usize);
    for y in 0..H {
        raw.push(0); // filter byte: None
        for x in 0..W {
            // Background: pale pink (#fef2f2). Border: red (#dc2626).
            // Diagonal X: red.
            let on_border = x < 4 || x >= W - 4 || y < 4 || y >= H - 4;
            let on_diag1 = x.abs_diff(y * W / H) < 4;
            let on_diag2 = x.abs_diff((H - 1 - y) * W / H) < 4;
            let (r, g, b) = if on_border || on_diag1 || on_diag2 {
                (0xdcu8, 0x26u8, 0x26u8)
            } else {
                (0xfeu8, 0xf2u8, 0xf2u8)
            };
            raw.push(r);
            raw.push(g);
            raw.push(b);
        }
    }
    let mut compressed: Vec<u8> = Vec::new();
    {
        let mut enc = ZlibEncoder::new(&mut compressed, Compression::default());
        enc.write_all(&raw).expect("zlib encode in-memory");
        enc.finish().expect("zlib finish");
    }

    let mut png: Vec<u8> = Vec::with_capacity(compressed.len() + 64);
    png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    // IHDR
    let mut ihdr_data = Vec::with_capacity(13);
    ihdr_data.extend_from_slice(&W.to_be_bytes());
    ihdr_data.extend_from_slice(&H.to_be_bytes());
    ihdr_data.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit RGB
    write_png_chunk(&mut png, b"IHDR", &ihdr_data);
    write_png_chunk(&mut png, b"IDAT", &compressed);
    write_png_chunk(&mut png, b"IEND", &[]);
    png
}

fn write_png_chunk(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(tag);
    out.extend_from_slice(data);
    let mut crc: u32 = 0xffff_ffff;
    for &b in tag.iter().chain(data.iter()) {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    out.extend_from_slice(&(!crc).to_be_bytes());
}

/// Fetch image bytes from an http(s) URL and parse them like a local file.
/// Times out at 10 seconds. Returns a helpful error if the optional
/// `remote-images` feature is disabled.
///
/// Flow: cache check → retry-loop fetch → sniff (validate) → cache write.
/// The sniff happens *before* the cache write so a 200 OK with garbage
/// (HTML error page, empty body) never poisons the cache.
#[cfg(feature = "remote-images")]
pub fn fetch_remote(url: &str) -> Result<ImageMeta> {
    let options = remote_image_options();
    let cache_path = remote_cache_path(url, &options);
    if let Some(path) = cache_path.as_deref() {
        if let Some(meta) = load_remote_cache(path, url) {
            return Ok(meta);
        }
    }

    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(10))
        .user_agent(&remote_user_agent(&options))
        .build();

    let bytes = fetch_with_retries(&agent, url, 3)?;
    let meta = sniff(&bytes, url)?;
    if let Some(path) = cache_path.as_deref() {
        write_cache(path, &bytes);
    }
    Ok(meta)
}

/// Runs the retry loop and returns the raw bytes on success. ureq converts
/// every non-2xx to `Error::Status`, so the `Ok(resp)` arm reliably means
/// 2xx. We don't retry on partial-read failure mid-stream — most failures
/// surface at connect/header time, and resuming would need Range support
/// from the server which isn't worth the complexity for v0.1.
#[cfg(feature = "remote-images")]
fn fetch_with_retries(agent: &ureq::Agent, url: &str, attempts: usize) -> Result<Vec<u8>> {
    use std::io::Read;

    const CAP_BYTES: u64 = 20 * 1024 * 1024;

    let mut last_err: anyhow::Error = anyhow!("fetch {}", url);
    for attempt in 0..attempts {
        match agent.get(url).call() {
            Ok(resp) => {
                // Read CAP_BYTES + 1 so we can distinguish "fits in cap"
                // from "would exceed cap, almost certainly truncated".
                let mut bytes: Vec<u8> = Vec::new();
                resp.into_reader()
                    .take(CAP_BYTES + 1)
                    .read_to_end(&mut bytes)
                    .with_context(|| format!("read {}", url))?;
                if bytes.len() as u64 > CAP_BYTES {
                    bail!(
                        "fetch {}: payload exceeds {} MB cap (received >{} bytes); \
                         host an optimised copy or vendor the image locally",
                        url,
                        CAP_BYTES / (1024 * 1024),
                        CAP_BYTES
                    );
                }
                return Ok(bytes);
            }
            Err(ureq::Error::Status(code, resp)) => {
                last_err = anyhow!("fetch {}: HTTP {}", url, code);
                let retryable = matches!(code, 408 | 429 | 500 | 502 | 503 | 504);
                if retryable && attempt + 1 < attempts {
                    std::thread::sleep(retry_delay(Some(&resp), attempt));
                    continue;
                }
                return Err(last_err);
            }
            Err(e) => {
                last_err = anyhow!("fetch {}: {}", url, e);
                if attempt + 1 < attempts {
                    std::thread::sleep(retry_delay(None, attempt));
                    continue;
                }
                return Err(last_err);
            }
        }
    }
    Err(last_err)
}

/// Best-effort cache write. Stages bytes under a process-unique `.tmp`
/// suffix so concurrent renders never clobber each other's staging file,
/// then renames into place. If rename fails (cross-filesystem move,
/// Windows already-exists), falls back to a direct write — no atomicity
/// but at least the cache entry exists.
#[cfg(feature = "remote-images")]
fn write_cache(path: &Path, bytes: &[u8]) {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = path.with_extension(format!("tmp.{}.{}", std::process::id(), nanos));
    if fs::write(&tmp, bytes).is_ok() {
        // rename → atomic on the same filesystem. Falls back to plain
        // write on Windows already-exists or cross-fs moves; we accept
        // the (very small) window of partial writes those scenarios open.
        if fs::rename(&tmp, path).is_err() {
            let _ = fs::write(path, bytes);
            let _ = fs::remove_file(&tmp);
        }
    }
}

#[cfg(feature = "remote-images")]
fn remote_user_agent(options: &RemoteImageOptions) -> String {
    options
        .user_agent
        .clone()
        .unwrap_or_else(|| format!("md2any/{}; remote image fetch", env!("CARGO_PKG_VERSION")))
}

/// Honours the `Retry-After` response header in either form (delta-seconds
/// or HTTP-date). Falls back to capped exponential backoff
/// (300ms × 2^attempt, max 30s) when no header is present or parseable.
/// All waits are clamped to 30s so a hostile / misconfigured server can't
/// stall the build for minutes.
#[cfg(feature = "remote-images")]
fn retry_delay(resp: Option<&ureq::Response>, attempt: usize) -> std::time::Duration {
    if let Some(header) = resp.and_then(|r| r.header("Retry-After")) {
        if let Some(secs) = parse_retry_after(header) {
            return std::time::Duration::from_secs(secs.min(30));
        }
    }
    let millis = 300u64.saturating_mul(1 << attempt.min(5));
    std::time::Duration::from_millis(millis.min(30_000))
}

/// Parse a `Retry-After` header value. Accepts two forms:
///   - delta-seconds: `Retry-After: 120`
///   - HTTP-date:     `Retry-After: Wed, 21 Oct 2015 07:28:00 GMT`
/// For the date form we compute (date - now) and clamp negative results
/// to zero. Returns `None` on anything else.
#[cfg(feature = "remote-images")]
fn parse_retry_after(value: &str) -> Option<u64> {
    let v = value.trim();
    if let Ok(secs) = v.parse::<u64>() {
        return Some(secs);
    }
    let target = parse_http_date(v)?;
    let now = std::time::SystemTime::now();
    target
        .duration_since(now)
        .ok()
        .map(|d| d.as_secs())
        .or(Some(0))
}

/// Tiny HTTP-date parser. Only handles the RFC 7231 "preferred" form:
///   `Sun, 06 Nov 1994 08:49:37 GMT`
/// That's the form modern servers emit. Obsolete RFC 850 and asctime forms
/// fall through to None and we use exponential backoff instead.
#[cfg(feature = "remote-images")]
fn parse_http_date(s: &str) -> Option<std::time::SystemTime> {
    // Format: `Sun, 06 Nov 1994 08:49:37 GMT`
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() != 6 || !parts[5].eq_ignore_ascii_case("GMT") {
        return None;
    }
    let day: u32 = parts[1].parse().ok()?;
    let month = match parts[2] {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let year: i32 = parts[3].parse().ok()?;
    let time: Vec<&str> = parts[4].split(':').collect();
    if time.len() != 3 {
        return None;
    }
    let hour: u32 = time[0].parse().ok()?;
    let minute: u32 = time[1].parse().ok()?;
    let second: u32 = time[2].parse().ok()?;

    // Days since Unix epoch (1970-01-01) using a small civil-from-days
    // calculation. Cribbed from Howard Hinnant's date algorithms.
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let doy =
        (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day.saturating_sub(1);
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era as i64 * 146097 + doe as i64 - 719468;
    let total_secs = days * 86400 + hour as i64 * 3600 + minute as i64 * 60 + second as i64;
    if total_secs < 0 {
        return None;
    }
    Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(total_secs as u64))
}

#[cfg(feature = "remote-images")]
fn load_remote_cache(path: &Path, url: &str) -> Option<ImageMeta> {
    let bytes = std::fs::read(path).ok()?;
    match sniff(&bytes, url) {
        Ok(meta) => Some(meta),
        Err(_) => {
            let _ = std::fs::remove_file(path);
            None
        }
    }
}

#[cfg(feature = "remote-images")]
fn remote_cache_path(url: &str, options: &RemoteImageOptions) -> Option<PathBuf> {
    if !options.cache_enabled {
        return None;
    }
    let root = options
        .cache_dir
        .clone()
        .unwrap_or_else(default_remote_cache_dir);
    let key = normalize_url_for_cache(url);
    Some(root.join(format!("{:016x}.img", fnv1a64(key.as_bytes()))))
}

/// Canonicalise the URL just enough that `foo`, `foo/`, `foo#anchor`, and
/// `foo` with stray whitespace hit the same cache slot. We deliberately do
/// **not** touch the query string (`?v=2` is a meaningful version key for
/// many hosts) and we don't case-fold path segments (some servers are
/// case-sensitive). Scheme + host are lowercased because they aren't.
#[cfg(feature = "remote-images")]
fn normalize_url_for_cache(url: &str) -> String {
    let trimmed = url.trim();
    // Drop fragment — never sent to server, irrelevant to image identity.
    let no_frag = trimmed.split('#').next().unwrap_or(trimmed);
    // Trim trailing slash from path-only URLs (not from query strings).
    let (base, query) = match no_frag.split_once('?') {
        Some((b, q)) => (b, Some(q)),
        None => (no_frag, None),
    };
    let base_norm = base.trim_end_matches('/');
    // Lower-case the scheme + host portion; leave the path alone.
    let (scheme_host, path) = match base_norm.find("://") {
        Some(idx) => {
            let after = &base_norm[idx + 3..];
            match after.find('/') {
                Some(slash) => (
                    base_norm[..idx + 3 + slash].to_ascii_lowercase(),
                    &after[slash..],
                ),
                None => (base_norm.to_ascii_lowercase(), ""),
            }
        }
        None => (String::new(), base_norm),
    };
    let mut out = scheme_host;
    out.push_str(path);
    if let Some(q) = query {
        out.push('?');
        out.push_str(q);
    }
    out
}

#[cfg(feature = "remote-images")]
fn default_remote_cache_dir() -> PathBuf {
    platform_cache_dir()
        .unwrap_or_else(|| std::env::temp_dir().join("md2any"))
        .join("remote-images")
}

/// Report the resolved remote-image cache directory and whether the
/// platform-standard location was found. Used by `md2any doctor` to flag
/// container/sandbox environments where the cache silently falls back to
/// the temp dir (and so doesn't survive a reboot).
#[cfg(feature = "remote-images")]
pub fn remote_cache_status() -> (PathBuf, bool) {
    let options = remote_image_options();
    let platform_ok = platform_cache_dir().is_some();
    let dir = options
        .cache_dir
        .clone()
        .unwrap_or_else(default_remote_cache_dir);
    (dir, platform_ok)
}

/// No-op stub when the `remote-images` feature is disabled — keeps the
/// `doctor` subcommand callable without a feature gate.
#[cfg(not(feature = "remote-images"))]
pub fn remote_cache_status() -> (PathBuf, bool) {
    (PathBuf::new(), false)
}

#[cfg(all(feature = "remote-images", target_os = "windows"))]
fn platform_cache_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(|p| PathBuf::from(p).join("md2any").join("Cache"))
        .or_else(|| {
            std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join("md2any").join("Cache"))
        })
}

#[cfg(all(feature = "remote-images", target_os = "macos"))]
fn platform_cache_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|p| {
        PathBuf::from(p)
            .join("Library")
            .join("Caches")
            .join("md2any")
    })
}

#[cfg(all(
    feature = "remote-images",
    not(any(target_os = "windows", target_os = "macos"))
))]
fn platform_cache_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CACHE_HOME")
        .map(|p| PathBuf::from(p).join("md2any"))
        .or_else(|| {
            std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".cache").join("md2any"))
        })
}

/// FNV-1a 64-bit, chosen for cache-key hashing over SHA-2 because:
///   - Inputs are URLs, never adversarial — no second-preimage threat
///   - Birthday-collision probability is ~10⁻¹² for a 10k-URL deck, which
///     means a collision is many orders of magnitude less likely than
///     someone deleting their cache by accident
///   - Zero dependencies, ~10 ns per URL, fits in one screen of code
/// If a deck ever grows to hundreds of millions of distinct URLs, switch
/// to SHA-256 — but you'll have other problems first.
#[cfg(feature = "remote-images")]
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(not(feature = "remote-images"))]
pub fn fetch_remote(url: &str) -> Result<ImageMeta> {
    bail!(
        "remote image {} requested but md2any was built without the \
         `remote-images` feature",
        url
    )
}

/// Sniff a byte buffer that's already in memory (used by both local + remote
/// load paths) and return metadata.
fn sniff(bytes: &[u8], origin: &str) -> Result<ImageMeta> {
    if bytes.len() < 16 {
        bail!("image too small: {}", origin);
    }
    if &bytes[..8] == b"\x89PNG\r\n\x1a\n" {
        let (w, h) = parse_png_dims(bytes).with_context(|| format!("parse PNG {}", origin))?;
        return Ok(ImageMeta {
            bytes: bytes.to_vec(),
            width: w,
            height: h,
            ext: "png",
        });
    }
    if bytes[0] == 0xFF && bytes[1] == 0xD8 {
        let (w, h) = parse_jpeg_dims(bytes).with_context(|| format!("parse JPEG {}", origin))?;
        return Ok(ImageMeta {
            bytes: bytes.to_vec(),
            width: w,
            height: h,
            ext: "jpeg",
        });
    }
    if looks_like_svg(bytes) {
        return rasterize_svg(bytes, origin);
    }
    bail!(
        "unsupported image format: {} (PNG, JPEG, and SVG supported)",
        origin
    )
}

fn looks_like_svg(bytes: &[u8]) -> bool {
    let prefix = std::str::from_utf8(&bytes[..bytes.len().min(512)]).unwrap_or("");
    let trimmed = prefix.trim_start();
    trimmed.starts_with("<svg") || (trimmed.starts_with("<?xml") && prefix.contains("<svg"))
}

/// Rasterise an SVG buffer to PNG via the resvg pipeline, then run it
/// through `sniff` recursively so the downstream renderers see a normal
/// PNG. Target DPI is fixed at 192 (2× retina) which keeps slide images
/// crisp without making them silly-large.
#[cfg(feature = "svg")]
fn rasterize_svg(bytes: &[u8], origin: &str) -> Result<ImageMeta> {
    // Build a font db once per SVG. Loading the bundled DejaVu families
    // guarantees text renders identically regardless of what's installed
    // on the machine running md2any.
    let mut fontdb = usvg::fontdb::Database::new();
    for ttf in crate::font::FONTS {
        fontdb.load_font_data(ttf.to_vec());
    }
    fontdb.set_sans_serif_family("DejaVu Sans");
    fontdb.set_monospace_family("DejaVu Sans Mono");
    let opt = usvg::Options {
        fontdb: std::sync::Arc::new(fontdb),
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_data(bytes, &opt)
        .map_err(|e| anyhow::anyhow!("parse SVG {}: {}", origin, e))?;
    let size = tree.size();
    let scale = 192.0 / 96.0; // raster at 2× display DPI
    let target_w = (size.width() * scale).ceil().max(1.0) as u32;
    let target_h = (size.height() * scale).ceil().max(1.0) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(target_w, target_h)
        .ok_or_else(|| anyhow::anyhow!("alloc {}x{} pixmap", target_w, target_h))?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale as f32, scale as f32),
        &mut pixmap.as_mut(),
    );
    let png = pixmap
        .encode_png()
        .map_err(|e| anyhow::anyhow!("encode PNG {}: {}", origin, e))?;
    sniff(&png, origin)
}

#[cfg(not(feature = "svg"))]
fn rasterize_svg(_bytes: &[u8], origin: &str) -> Result<ImageMeta> {
    bail!(
        "SVG image {} requested but md2any was built without the `svg` feature",
        origin
    )
}

/// Read an image file, sniff its format, and parse just enough of the
/// container to learn its pixel dimensions. PNG and JPEG embed directly;
/// SVG is rasterised to PNG via the `svg` feature.
pub fn load(path: &Path) -> Result<ImageMeta> {
    let bytes = std::fs::read(path).with_context(|| format!("read image {}", path.display()))?;
    sniff(&bytes, &path.display().to_string())
}

fn parse_png_dims(b: &[u8]) -> Result<(u32, u32)> {
    if b.len() < 24 {
        bail!("truncated PNG header");
    }
    let w = u32::from_be_bytes([b[16], b[17], b[18], b[19]]);
    let h = u32::from_be_bytes([b[20], b[21], b[22], b[23]]);
    if w == 0 || h == 0 {
        bail!("PNG reports zero dimensions");
    }
    Ok((w, h))
}

fn parse_jpeg_dims(b: &[u8]) -> Result<(u32, u32)> {
    let mut i = 2;
    while i + 1 < b.len() {
        while i < b.len() && b[i] == 0xFF {
            i += 1;
        }
        if i >= b.len() {
            bail!("JPEG truncated before marker");
        }
        let marker = b[i];
        i += 1;
        if matches!(
            marker,
            0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF
        ) {
            if i + 7 > b.len() {
                bail!("JPEG SOF truncated");
            }
            let h = u16::from_be_bytes([b[i + 3], b[i + 4]]) as u32;
            let w = u16::from_be_bytes([b[i + 5], b[i + 6]]) as u32;
            if w == 0 || h == 0 {
                bail!("JPEG reports zero dimensions");
            }
            return Ok((w, h));
        }
        if matches!(marker, 0xD0..=0xD9) {
            continue;
        }
        if i + 2 > b.len() {
            bail!("JPEG length truncated");
        }
        let seg_len = u16::from_be_bytes([b[i], b[i + 1]]) as usize;
        if seg_len < 2 {
            bail!("invalid JPEG segment length");
        }
        i += seg_len;
    }
    bail!("JPEG SOF not found")
}

#[cfg(all(test, feature = "remote-images"))]
mod tests {
    use super::*;

    #[test]
    fn fnv1a64_known_vectors() {
        // FNV-1a 64-bit reference values from the canonical spec.
        assert_eq!(fnv1a64(b""), 0xcbf29ce484222325);
        assert_eq!(fnv1a64(b"a"), 0xaf63dc4c8601ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x85944171f73967e8);
    }

    #[test]
    fn fnv1a64_collisions_unlikely_for_typical_input() {
        // Sanity: 1,000 sequentially-numbered URLs should all hash to
        // distinct keys. A real collision here would indicate a bug.
        let mut seen = std::collections::HashSet::new();
        for i in 0..1_000 {
            let url = format!("https://example.com/image-{}.png", i);
            assert!(seen.insert(fnv1a64(url.as_bytes())));
        }
    }

    #[test]
    fn normalise_strips_fragment_and_trailing_slash() {
        assert_eq!(
            normalize_url_for_cache("https://example.com/foo/"),
            normalize_url_for_cache("https://example.com/foo"),
        );
        assert_eq!(
            normalize_url_for_cache("https://example.com/foo#anchor"),
            normalize_url_for_cache("https://example.com/foo"),
        );
        assert_eq!(
            normalize_url_for_cache("  https://example.com/foo  "),
            normalize_url_for_cache("https://example.com/foo"),
        );
    }

    #[test]
    fn normalise_lowercases_scheme_and_host_only() {
        // Path is case-sensitive on most webservers; query is meaningful.
        assert_eq!(
            normalize_url_for_cache("HTTPS://Example.COM/Path?V=2"),
            "https://example.com/Path?V=2",
        );
    }

    #[test]
    fn normalise_keeps_query_string() {
        // ?v=2 is a meaningful cache-busting marker.
        let a = normalize_url_for_cache("https://example.com/img.png?v=1");
        let b = normalize_url_for_cache("https://example.com/img.png?v=2");
        assert_ne!(a, b);
    }

    #[test]
    fn parse_retry_after_seconds() {
        assert_eq!(parse_retry_after("30"), Some(30));
        assert_eq!(parse_retry_after("  120  "), Some(120));
        assert_eq!(parse_retry_after("0"), Some(0));
    }

    #[test]
    fn parse_retry_after_garbage_is_none() {
        assert_eq!(parse_retry_after("soon"), None);
        assert_eq!(parse_retry_after(""), None);
    }

    #[test]
    fn parse_retry_after_date_form() {
        // Date far in the future → some positive delta.
        let delta = parse_retry_after("Wed, 31 Dec 2099 23:59:59 GMT").unwrap();
        assert!(delta > 0);
    }

    #[test]
    fn parse_retry_after_date_in_past_is_zero() {
        // Date in the past should clamp to 0 (server's "you can retry now").
        assert_eq!(parse_retry_after("Sun, 06 Nov 1994 08:49:37 GMT"), Some(0));
    }

    #[test]
    fn retry_delay_caps_at_30s() {
        // attempt=100 with no header → exponential 300ms × 2^min(100,5) =
        // 300ms × 32 = 9.6s, well under cap.
        let d = retry_delay(None, 100);
        assert!(d.as_secs() <= 30);
    }
}
