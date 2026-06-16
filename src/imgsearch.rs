//! Image search across free and (optionally) keyed providers, for the
//! `--serve --edit` AI dock. Returns license-aware candidates so the model can
//! insert a *real* photo (md2any then downloads the URL at render time).
//!
//! Keyless sources — Wikimedia Commons and Openverse — are always queried.
//! Keyed sources — Unsplash and Pexels — are queried only when a key file is
//! present (`unsplash-api.key` / `pexels-api.key`, same convention as
//! `grok-api.key`), so "all sources" lights up automatically once keyed.

/// One image search result with the metadata needed to insert it *and*
/// attribute it. `image_url` is a directly-fetchable full-resolution URL.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ImageHit {
    pub title: String,
    pub image_url: String,
    pub thumb_url: String,
    pub license: String,
    pub author: String,
    pub source: String,
    pub page_url: String,
}

const UA: &str = "md2any/0.4 image search (+https://github.com/JavaPerformance/md2any)";

/// Percent-encode a query for a URL query-string value.
fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Strip HTML tags (Commons returns Artist/license as small HTML fragments).
fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.trim().to_string()
}

/// Trim a natural-language request down to a tight image query: drop filler
/// words and cap length. Commons full-text search ANDs every word, so
/// "real photo of a Zilog Z80 chip package" finds nothing while "Zilog Z80
/// chip" finds plenty.
fn normalize_query(q: &str) -> String {
    const FILLER: &[&str] = &[
        "a",
        "an",
        "the",
        "of",
        "for",
        "real",
        "photo",
        "photograph",
        "image",
        "picture",
        "pic",
        "shot",
        "find",
        "please",
        "add",
        "showing",
        "show",
        "close",
        "up",
        "closeup",
        "high",
        "res",
        "resolution",
        "with",
    ];
    let words: Vec<&str> = q
        .split_whitespace()
        .filter(|w| {
            let lw = w
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_ascii_lowercase();
            !lw.is_empty() && !FILLER.contains(&lw.as_str())
        })
        .take(6)
        .collect();
    if words.is_empty() {
        q.trim().to_string()
    } else {
        words.join(" ")
    }
}

#[cfg(feature = "ureq")]
fn read_key(file: &str) -> Option<String> {
    std::fs::read_to_string(file)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Search every available source, newest/most-relevant first per source,
/// interleaved. `per_source` caps results from each provider.
#[cfg(feature = "ureq")]
pub fn search(query: &str, per_source: usize) -> Vec<ImageHit> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(12))
        .user_agent(UA)
        .build();
    let n = per_source.clamp(1, 10);
    let query = &normalize_query(query);
    let mut hits = Vec::new();
    hits.extend(commons(&agent, query, n).unwrap_or_default());
    hits.extend(openverse(&agent, query, n).unwrap_or_default());
    if let Some(key) = read_key("unsplash-api.key") {
        hits.extend(unsplash(&agent, query, n, &key).unwrap_or_default());
    }
    if let Some(key) = read_key("pexels-api.key") {
        hits.extend(pexels(&agent, query, n, &key).unwrap_or_default());
    }
    // md2any embeds only JPEG/PNG/SVG. Drop anything else, but first try the
    // provider's thumbnail (Commons renders webp/tiff originals to a JPEG
    // thumb), so a great photo in an unsupported original format still works.
    hits.retain_mut(|h| {
        if supported(&h.image_url) {
            true
        } else if supported(&h.thumb_url) {
            h.image_url = h.thumb_url.clone();
            true
        } else {
            false
        }
    });
    hits
}

/// True if md2any can embed this image URL (by extension): JPEG/PNG/SVG/WebP
/// (WebP is decoded + re-encoded when the `webp` feature is on, the default).
fn supported(url: &str) -> bool {
    let u = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    [".jpg", ".jpeg", ".png", ".svg", ".webp"]
        .iter()
        .any(|e| u.ends_with(e))
}

#[cfg(not(feature = "ureq"))]
pub fn search(_query: &str, _per_source: usize) -> Vec<ImageHit> {
    Vec::new()
}

#[cfg(feature = "ureq")]
fn get_json(agent: &ureq::Agent, url: &str, headers: &[(&str, &str)]) -> Option<serde_json::Value> {
    let mut req = agent.get(url);
    for (k, v) in headers {
        req = req.set(k, v);
    }
    let text = req.call().ok()?.into_string().ok()?;
    serde_json::from_str(&text).ok()
}

/// Commons ANDs every search word, so a too-specific query finds nothing.
/// Try the full query, then progressively shorter prefixes, until one hits.
#[cfg(feature = "ureq")]
fn commons(agent: &ureq::Agent, query: &str, n: usize) -> Option<Vec<ImageHit>> {
    let words: Vec<&str> = query.split_whitespace().collect();
    let mut seen = std::collections::HashSet::new();
    for take in [words.len(), 3, 2] {
        let take = take.min(words.len()).max(1);
        let q = words[..take].join(" ");
        if !seen.insert(q.clone()) {
            continue;
        }
        if let Some(hits) = commons_once(agent, &q, n) {
            if !hits.is_empty() {
                return Some(hits);
            }
        }
    }
    Some(Vec::new())
}

#[cfg(feature = "ureq")]
fn commons_once(agent: &ureq::Agent, query: &str, n: usize) -> Option<Vec<ImageHit>> {
    let url = format!(
        "https://commons.wikimedia.org/w/api.php?action=query&generator=search\
         &gsrsearch={}&gsrnamespace=6&gsrlimit={n}&prop=imageinfo\
         &iiprop=url%7Cextmetadata&iiurlwidth=1280&format=json",
        enc(query)
    );
    let v = get_json(agent, &url, &[])?;
    let pages = v.get("query")?.get("pages")?.as_object()?;
    let is_image = |u: &str| {
        let u = u.to_ascii_lowercase();
        [".jpg", ".jpeg", ".png", ".gif", ".webp", ".svg"]
            .iter()
            .any(|e| u.ends_with(e))
    };
    let mut out = Vec::new();
    for page in pages.values() {
        let Some(info) = page.get("imageinfo").and_then(|a| a.get(0)) else {
            continue;
        };
        let Some(image_url) = info.get("url").and_then(|u| u.as_str()) else {
            continue;
        };
        // Namespace 6 includes PDFs/video/audio; keep only embeddable images.
        if !is_image(image_url) {
            continue;
        }
        let image_url = image_url.to_string();
        let thumb = info
            .get("thumburl")
            .and_then(|t| t.as_str())
            .unwrap_or(&image_url)
            .to_string();
        let em = info.get("extmetadata");
        let lic = em
            .and_then(|e| e.get("LicenseShortName"))
            .and_then(|l| l.get("value"))
            .and_then(|s| s.as_str())
            .unwrap_or("see Commons");
        let artist = em
            .and_then(|e| e.get("Artist"))
            .and_then(|a| a.get("value"))
            .and_then(|s| s.as_str())
            .map(strip_html)
            .unwrap_or_default();
        out.push(ImageHit {
            title: page
                .get("title")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .trim_start_matches("File:")
                .to_string(),
            image_url,
            thumb_url: thumb,
            license: lic.to_string(),
            author: artist,
            source: "Wikimedia Commons".into(),
            page_url: format!(
                "https://commons.wikimedia.org/wiki/{}",
                enc(page.get("title").and_then(|t| t.as_str()).unwrap_or(""))
            ),
        });
    }
    Some(out)
}

#[cfg(feature = "ureq")]
fn openverse(agent: &ureq::Agent, query: &str, n: usize) -> Option<Vec<ImageHit>> {
    let url = format!(
        "https://api.openverse.org/v1/images/?q={}&page_size={n}",
        enc(query)
    );
    let v = get_json(agent, &url, &[])?;
    let results = v.get("results")?.as_array()?;
    Some(
        results
            .iter()
            .filter_map(|r| {
                let image_url = r.get("url")?.as_str()?.to_string();
                Some(ImageHit {
                    title: r
                        .get("title")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string(),
                    thumb_url: r
                        .get("thumbnail")
                        .and_then(|t| t.as_str())
                        .unwrap_or(&image_url)
                        .to_string(),
                    image_url,
                    license: r
                        .get("license")
                        .and_then(|l| l.as_str())
                        .map(|l| format!("CC {}", l.to_uppercase()))
                        .unwrap_or_else(|| "CC".into()),
                    author: r
                        .get("creator")
                        .and_then(|c| c.as_str())
                        .unwrap_or("")
                        .to_string(),
                    source: "Openverse".into(),
                    page_url: r
                        .get("foreign_landing_url")
                        .and_then(|u| u.as_str())
                        .unwrap_or("")
                        .to_string(),
                })
            })
            .collect(),
    )
}

#[cfg(feature = "ureq")]
fn unsplash(agent: &ureq::Agent, query: &str, n: usize, key: &str) -> Option<Vec<ImageHit>> {
    let url = format!(
        "https://api.unsplash.com/search/photos?query={}&per_page={n}",
        enc(query)
    );
    let v = get_json(
        agent,
        &url,
        &[("Authorization", &format!("Client-ID {key}"))],
    )?;
    let results = v.get("results")?.as_array()?;
    Some(
        results
            .iter()
            .filter_map(|r| {
                Some(ImageHit {
                    title: r
                        .get("description")
                        .and_then(|d| d.as_str())
                        .or_else(|| r.get("alt_description").and_then(|d| d.as_str()))
                        .unwrap_or("Unsplash photo")
                        .to_string(),
                    image_url: r.get("urls")?.get("regular")?.as_str()?.to_string(),
                    thumb_url: r
                        .get("urls")
                        .and_then(|u| u.get("small"))
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string(),
                    license: "Unsplash License".into(),
                    author: r
                        .get("user")
                        .and_then(|u| u.get("name"))
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string(),
                    source: "Unsplash".into(),
                    page_url: r
                        .get("links")
                        .and_then(|l| l.get("html"))
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string(),
                })
            })
            .collect(),
    )
}

#[cfg(feature = "ureq")]
fn pexels(agent: &ureq::Agent, query: &str, n: usize, key: &str) -> Option<Vec<ImageHit>> {
    let url = format!(
        "https://api.pexels.com/v1/search?query={}&per_page={n}",
        enc(query)
    );
    let v = get_json(agent, &url, &[("Authorization", key)])?;
    let photos = v.get("photos")?.as_array()?;
    Some(
        photos
            .iter()
            .filter_map(|p| {
                Some(ImageHit {
                    title: p
                        .get("alt")
                        .and_then(|a| a.as_str())
                        .unwrap_or("Pexels photo")
                        .to_string(),
                    image_url: p.get("src")?.get("large")?.as_str()?.to_string(),
                    thumb_url: p
                        .get("src")
                        .and_then(|s| s.get("medium"))
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string(),
                    license: "Pexels License".into(),
                    author: p
                        .get("photographer")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string(),
                    source: "Pexels".into(),
                    page_url: p
                        .get("url")
                        .and_then(|u| u.as_str())
                        .unwrap_or("")
                        .to_string(),
                })
            })
            .collect(),
    )
}
