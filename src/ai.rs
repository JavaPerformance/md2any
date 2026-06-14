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

/// Extract the assistant's message text from a chat-completions response, or a
/// useful error if the body is an API error / unexpected shape.
pub fn parse_response(body: &str) -> Result<String, String> {
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
        .map(|s| strip_fences(s.trim()).to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "API response had no message content".to_string())
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
}
