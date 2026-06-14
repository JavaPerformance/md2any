//! Optional AI deck generation.
//!
//! `--generate "<prompt>"` asks a chat model to draft md2any-flavoured markdown
//! which is then rendered through the normal pipeline. The client speaks the
//! widely-supported OpenAI-style `/v1/chat/completions` shape, so it works with
//! most providers and gateways — endpoint, model, and key are all configurable;
//! nothing about a specific vendor is required.
//!
//! Request/response handling is pure and unit-tested; only [`generate`] touches
//! the network (behind the `ureq` dependency that the `ai` feature pulls in).

/// The default chat endpoint and model. Both are overridable via
/// `--ai-endpoint` / `--ai-model`, so this is just a convenience preset.
pub const DEFAULT_ENDPOINT: &str = "https://api.openai.com/v1/chat/completions";
pub const DEFAULT_MODEL: &str = "gpt-4o-mini";

/// Environment variables consulted for the API key, in order.
pub const KEY_ENVS: [&str; 2] = ["MD2ANY_API_KEY", "OPENAI_API_KEY"];

/// A provider preset: a gitignored key-file name plus the endpoint and model to
/// use when that file supplies the key. Dropping the matching file in the
/// working directory switches providers with no flags; `--ai-endpoint` /
/// `--ai-model` still override. Listed in priority order. Nothing here bakes in
/// a single vendor — it's just a convenience table over the same OpenAI-style
/// wire format every entry speaks.
pub struct Provider {
    pub key_file: &'static str,
    pub endpoint: &'static str,
    pub model: &'static str,
}

pub const PROVIDERS: &[Provider] = &[
    Provider {
        key_file: "grok-api.key",
        endpoint: "https://api.x.ai/v1/chat/completions",
        model: "grok-4.3",
    },
    Provider {
        key_file: "md2any-openai-api.key",
        endpoint: DEFAULT_ENDPOINT,
        model: DEFAULT_MODEL,
    },
    Provider {
        key_file: ".md2any.key",
        endpoint: DEFAULT_ENDPOINT,
        model: DEFAULT_MODEL,
    },
    Provider {
        key_file: "md2any.key",
        endpoint: DEFAULT_ENDPOINT,
        model: DEFAULT_MODEL,
    },
];

#[derive(Debug, Clone)]
pub struct AiOptions {
    pub endpoint: String,
    pub model: String,
    pub api_key: String,
}

impl AiOptions {
    /// Build options from explicit endpoint/model (falling back to the presets)
    /// and an API key read from [`KEY_ENVS`]. Errors if no key is set.
    pub fn from_env(endpoint: Option<String>, model: Option<String>) -> Result<Self, String> {
        let api_key = KEY_ENVS
            .iter()
            .find_map(|k| std::env::var(k).ok().filter(|v| !v.trim().is_empty()))
            .ok_or_else(|| {
                format!(
                    "no API key found — set one of {} (or pipe your own markdown instead)",
                    KEY_ENVS.join(" / ")
                )
            })?;
        Ok(AiOptions {
            endpoint: endpoint.unwrap_or_else(|| DEFAULT_ENDPOINT.to_string()),
            model: model.unwrap_or_else(|| DEFAULT_MODEL.to_string()),
            api_key,
        })
    }

    /// Like [`from_env`](Self::from_env) but also falls back to the first
    /// readable [`PROVIDERS`] key file in the working directory, adopting that
    /// provider's endpoint and model (unless the caller passed overrides). Used
    /// by the `--serve --edit` AI dock so a key can live in a gitignored file.
    pub fn resolve(endpoint: Option<String>, model: Option<String>) -> Result<Self, String> {
        // An explicit env key keeps the OpenAI-style defaults (or overrides).
        if let Some(api_key) = KEY_ENVS
            .iter()
            .find_map(|k| std::env::var(k).ok().filter(|v| !v.trim().is_empty()))
        {
            return Ok(AiOptions {
                endpoint: endpoint.unwrap_or_else(|| DEFAULT_ENDPOINT.to_string()),
                model: model.unwrap_or_else(|| DEFAULT_MODEL.to_string()),
                api_key,
            });
        }
        // Otherwise the first readable provider key file, with its presets.
        for p in PROVIDERS {
            if let Some(api_key) = std::fs::read_to_string(p.key_file)
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
            {
                return Ok(AiOptions {
                    endpoint: endpoint.unwrap_or_else(|| p.endpoint.to_string()),
                    model: model.unwrap_or_else(|| p.model.to_string()),
                    api_key,
                });
            }
        }
        Err(format!(
            "no API key — set {} or create one of {}",
            KEY_ENVS.join(" / "),
            PROVIDERS
                .iter()
                .map(|p| p.key_file)
                .collect::<Vec<_>>()
                .join(" / ")
        ))
    }
}

/// The instruction given to the model. Produces a deck in md2any's markdown
/// conventions so the output drops straight into the renderer.
pub fn system_prompt() -> String {
    "You are a slide-deck author. Output ONLY GitHub-flavoured markdown for a \
     presentation — no commentary, no code fence around the whole document. \
     Conventions: an optional YAML front-matter block at the very top \
     (between --- fences) may set `title`, `subtitle`, `author`, `theme: \
     light|dark`, and `aspect: 16:9`. Use a single `#` heading as a section \
     divider and `##` headings to start each content slide. Keep each slide \
     focused: a short title and a handful of bullet points, a small table, or \
     a fenced code block. Use `$...$`/`$$...$$` for math when relevant. Aim \
     for a coherent, well-paced deck."
        .to_string()
}

/// System prompt for the `--serve --edit` AI dock. Unlike [`system_prompt`]
/// (one-shot deck generation), this teaches the model md2any's full markup and
/// an edit protocol so it can both answer questions about the open document and
/// rewrite it on request.
pub fn editor_system_prompt() -> String {
    r#"You are the writing assistant built into md2any, a tool that turns one
markdown source into slide decks (PPTX/PDF/HTML/SVG and more). The user is
editing a markdown deck in a live editor; you can discuss it and rewrite it.

md2any markup you must know:
- Optional YAML front-matter between `---` fences at the very top. Keys include:
  `title`, `subtitle`, `author`, `date`, `theme` (light, dark, corporate, sepia,
  contrast, midnight, terminal, pastel), `aspect` (16:9, 4:3, 16:10),
  `transition` (none, fade, push, wipe, cover), `toc: true`, `logo`,
  `math` (unicode | source | svg), and a `style:` block (an inline theme
  override). In `style:`, colours MUST be hex like `#22D3EE` (CSS names such as
  `cyan` are rejected): `accent`, `bg`, `title_color`, `body_color`; plus
  integer point sizes `title_size`/`body_size` and font-family names.
- `# Heading` is a SECTION divider slide. `## Heading` starts a CONTENT slide.
  A `---` horizontal rule also starts a new content slide. The deck's first
  slide is a title slide built from the front-matter.
- Body of a slide: paragraphs, `-`/`1.` lists (indent for nesting),
  `> blockquotes`, GFM tables (with `:---`, `:---:`, `---:` alignment),
  fenced code blocks (```lang), and images `![alt](path){width=60%}`.
- Two-column layout: wrap content with `:::` sentinels — left, then `:::`,
  then right, then a closing `:::`.
- Math: `$inline$` and `$$display$$`, LaTeX-ish (\frac, \sqrt, ^, _, \sum,
  Greek, \mathbf, accents like \bar, \hat, \vec). Set `math: svg` in
  front-matter for crisp rendered equations.
- Speaker notes: `<!-- notes: ... -->`. Per-slide background: `<!-- bg: path -->`.
  Layout hints: `<!-- layout: image-left|image-right|image-full|text-full -->`.

EDIT PROTOCOL — follow exactly:
- If the user asks you to change the deck, reply with a one or two sentence
  summary of what you changed, then the COMPLETE updated document wrapped in a
  fenced block that OPENS with a line of FOUR backticks followed by `md2any`
  (````md2any) and CLOSES with a line of four backticks. Four backticks are
  required so the document can itself contain normal ``` code blocks without
  ending the block early. Include the front-matter; do not abbreviate or use
  placeholders like "...": output the whole file so it can be applied verbatim.
- If the user only asks a question or for advice, answer normally and DO NOT
  include a document block.
- Keep slides focused and well-paced; preserve the user's voice and existing
  content unless asked to change it."#
        .to_string()
}

/// JSON body for an OpenAI-style chat-completions request.
pub fn build_request_body(model: &str, system: &str, user: &str) -> String {
    serde_json::json!({
        "model": model,
        "temperature": 0.7,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user },
        ],
    })
    .to_string()
}

/// Build a chat-completions body from a full message array (role/content
/// objects). Used by the editor dock to carry conversation history.
pub fn chat_request_body(model: &str, messages: &[serde_json::Value]) -> String {
    serde_json::json!({
        "model": model,
        "temperature": 0.5,
        "messages": messages,
    })
    .to_string()
}

/// As [`chat_request_body`] but with `stream: true`, so the provider returns
/// incremental SSE deltas.
pub fn chat_request_body_stream(model: &str, messages: &[serde_json::Value]) -> String {
    serde_json::json!({
        "model": model,
        "temperature": 0.5,
        "stream": true,
        "messages": messages,
    })
    .to_string()
}

/// Pull the incremental text out of one streamed `data:` payload (a chat
/// completion chunk). Returns `Ok(Some(delta))` for content, `Ok(None)` for
/// chunks with no text (role headers, finish markers), or `Err` for an API
/// error embedded in the stream.
pub fn parse_stream_chunk(payload: &str) -> Result<Option<String>, String> {
    let v: serde_json::Value = match serde_json::from_str(payload) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    if let Some(msg) = v
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
    {
        return Err(format!("API error: {msg}"));
    }
    Ok(v.get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("delta"))
        .and_then(|d| d.get("content"))
        .and_then(|t| t.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string()))
}

/// Extract the assistant's message text from a chat-completions response, or a
/// useful error if the body is an API error / unexpected shape. Unlike
/// [`parse_response`], the content is returned verbatim (fences intact) — the
/// editor dock needs the raw reply to detect an embedded ```md2any block.
pub fn parse_chat_response(body: &str) -> Result<String, String> {
    let json: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("invalid JSON from API: {e}"))?;
    if let Some(msg) = json
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
    {
        return Err(format!("API error: {msg}"));
    }
    json.get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|t| t.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "API response had no message content".to_string())
}

/// Extract the assistant's message text from a chat-completions response, or a
/// useful error if the body is an API error / unexpected shape.
pub fn parse_response(body: &str) -> Result<String, String> {
    parse_chat_response(body).map(|s| strip_fences(s.trim()).to_string())
}

/// If the model wrapped the whole reply in a ```markdown fence, unwrap it.
fn strip_fences(s: &str) -> &str {
    let t = s.trim();
    if let Some(rest) = t.strip_prefix("```") {
        // Drop the info string on the opening fence line.
        let after_open = rest.splitn(2, '\n').nth(1).unwrap_or("");
        if let Some(inner) = after_open.rfind("```") {
            return after_open[..inner].trim();
        }
    }
    t
}

/// Generate a markdown deck for `prompt`. Network call; needs the `ureq`
/// dependency (pulled in by the default `ai` feature).
#[cfg(feature = "ureq")]
pub fn generate(opts: &AiOptions, prompt: &str) -> Result<String, String> {
    let body = build_request_body(&opts.model, &system_prompt(), prompt);
    let resp = ureq::post(&opts.endpoint)
        .set("Authorization", &format!("Bearer {}", opts.api_key))
        .set("Content-Type", "application/json")
        .send_string(&body);
    match resp {
        Ok(r) => {
            let text = r
                .into_string()
                .map_err(|e| format!("read API response: {e}"))?;
            parse_response(&text)
        }
        Err(ureq::Error::Status(code, r)) => {
            let detail = r.into_string().unwrap_or_default();
            // Surface the API's own error message when present.
            match parse_response(&detail) {
                Err(msg) if msg.starts_with("API error:") => Err(msg),
                _ => Err(format!("API returned HTTP {code}: {}", detail.trim())),
            }
        }
        Err(e) => Err(format!("request to {} failed: {e}", opts.endpoint)),
    }
}

#[cfg(not(feature = "ureq"))]
pub fn generate(_opts: &AiOptions, _prompt: &str) -> Result<String, String> {
    Err(
        "this build has no network support (rebuild with the default features \
         or the `ai` feature)"
            .to_string(),
    )
}

/// Send a full message array (system + conversation) and return the assistant's
/// raw reply. Used by the editor's AI dock. Network call; needs `ureq`.
#[cfg(feature = "ureq")]
pub fn chat(opts: &AiOptions, messages: &[serde_json::Value]) -> Result<String, String> {
    let body = chat_request_body(&opts.model, messages);
    let resp = ureq::post(&opts.endpoint)
        .set("Authorization", &format!("Bearer {}", opts.api_key))
        .set("Content-Type", "application/json")
        .send_string(&body);
    match resp {
        Ok(r) => {
            let text = r
                .into_string()
                .map_err(|e| format!("read API response: {e}"))?;
            parse_chat_response(&text)
        }
        Err(ureq::Error::Status(code, r)) => {
            let detail = r.into_string().unwrap_or_default();
            match parse_chat_response(&detail) {
                Err(msg) if msg.starts_with("API error:") => Err(msg),
                _ => Err(format!("API returned HTTP {code}: {}", detail.trim())),
            }
        }
        Err(e) => Err(format!("request to {} failed: {e}", opts.endpoint)),
    }
}

#[cfg(not(feature = "ureq"))]
pub fn chat(_opts: &AiOptions, _messages: &[serde_json::Value]) -> Result<String, String> {
    Err("this build has no network support (rebuild with the `ai` feature)".to_string())
}

/// Stream a chat completion, invoking `on_delta` with each text fragment as it
/// arrives. Reads the provider's SSE (`data: {…}` lines). `on_delta` returning
/// `Err` (e.g. the client disconnected) aborts the stream. Network call; `ureq`.
#[cfg(feature = "ureq")]
pub fn chat_stream(
    opts: &AiOptions,
    messages: &[serde_json::Value],
    on_delta: &mut dyn FnMut(&str) -> Result<(), String>,
) -> Result<(), String> {
    use std::io::BufRead;
    let body = chat_request_body_stream(&opts.model, messages);
    let resp = ureq::post(&opts.endpoint)
        .set("Authorization", &format!("Bearer {}", opts.api_key))
        .set("Content-Type", "application/json")
        .send_string(&body);
    let reader = match resp {
        Ok(r) => std::io::BufReader::new(r.into_reader()),
        Err(ureq::Error::Status(code, r)) => {
            let detail = r.into_string().unwrap_or_default();
            return match parse_chat_response(&detail) {
                Err(msg) if msg.starts_with("API error:") => Err(msg),
                _ => Err(format!("API returned HTTP {code}: {}", detail.trim())),
            };
        }
        Err(e) => return Err(format!("request to {} failed: {e}", opts.endpoint)),
    };
    for line in reader.lines() {
        let line = line.map_err(|e| format!("stream read: {e}"))?;
        let Some(payload) = line.strip_prefix("data:") else {
            continue;
        };
        let payload = payload.trim();
        if payload.is_empty() {
            continue;
        }
        if payload == "[DONE]" {
            break;
        }
        if let Some(delta) = parse_stream_chunk(payload)? {
            on_delta(&delta)?;
        }
    }
    Ok(())
}

#[cfg(not(feature = "ureq"))]
pub fn chat_stream(
    _opts: &AiOptions,
    _messages: &[serde_json::Value],
    _on_delta: &mut dyn FnMut(&str) -> Result<(), String>,
) -> Result<(), String> {
    Err("this build has no network support (rebuild with the `ai` feature)".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_body_has_model_and_roles() {
        let body = build_request_body("m1", "sys", "hello");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["model"], "m1");
        assert_eq!(v["messages"][0]["role"], "system");
        assert_eq!(v["messages"][1]["content"], "hello");
    }

    #[test]
    fn parse_extracts_content_and_strips_fence() {
        let body = r##"{"choices":[{"message":{"content":"```markdown\n# Hi\n\n- a\n```"}}]}"##;
        assert_eq!(parse_response(body).unwrap(), "# Hi\n\n- a");
        let plain = r##"{"choices":[{"message":{"content":"# Plain\n- x"}}]}"##;
        assert_eq!(parse_response(plain).unwrap(), "# Plain\n- x");
    }

    #[test]
    fn parse_surfaces_api_errors() {
        let body = r#"{"error":{"message":"bad key"}}"#;
        assert_eq!(parse_response(body).unwrap_err(), "API error: bad key");
        assert!(parse_response("not json").is_err());
        assert!(parse_response(r#"{"choices":[]}"#).is_err());
    }

    #[test]
    fn chat_body_carries_full_message_array() {
        let msgs = vec![
            serde_json::json!({ "role": "system", "content": "sys" }),
            serde_json::json!({ "role": "user", "content": "hi" }),
            serde_json::json!({ "role": "assistant", "content": "yo" }),
        ];
        let v: serde_json::Value = serde_json::from_str(&chat_request_body("m", &msgs)).unwrap();
        assert_eq!(v["messages"].as_array().unwrap().len(), 3);
        assert_eq!(v["messages"][2]["role"], "assistant");
    }

    #[test]
    fn chat_response_keeps_fences_for_dock() {
        // The dock must see the raw reply (fence intact) to extract a doc block,
        // unlike parse_response which unwraps a single whole-reply fence.
        let body = r##"{"choices":[{"message":{"content":"done.\n\n````md2any\n# Hi\n````"}}]}"##;
        let raw = parse_chat_response(body).unwrap();
        assert!(raw.contains("````md2any"), "{raw}");
        assert!(raw.starts_with("done."));
    }

    #[test]
    fn stream_chunk_extracts_delta_and_errors() {
        let chunk = r#"{"choices":[{"delta":{"content":"Hel"}}]}"#;
        assert_eq!(parse_stream_chunk(chunk).unwrap(), Some("Hel".to_string()));
        // role-only / empty deltas yield None (nothing to append)
        let role = r#"{"choices":[{"delta":{"role":"assistant"}}]}"#;
        assert_eq!(parse_stream_chunk(role).unwrap(), None);
        // an error embedded mid-stream surfaces as Err
        let err = r#"{"error":{"message":"rate limited"}}"#;
        assert_eq!(
            parse_stream_chunk(err).unwrap_err(),
            "API error: rate limited"
        );
        // non-JSON keepalive lines are ignored
        assert_eq!(parse_stream_chunk("ka").unwrap(), None);
    }

    #[test]
    fn editor_prompt_documents_markup_and_protocol() {
        let p = editor_system_prompt();
        assert!(p.contains("md2any"));
        assert!(p.contains("front-matter"));
        assert!(p.contains("md2any")); // edit-protocol fence tag
    }
}
