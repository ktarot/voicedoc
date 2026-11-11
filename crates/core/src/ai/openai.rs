//! OpenAI Chat Completions API.

use serde_json::{json, Value};

use super::{api_error, read_sse, AiConfig, AiOutput};
use crate::{Cancel, Error, Result};

pub const DEFAULT_MODEL: &str = "gpt-5";
pub const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

pub fn complete(
    cfg: &AiConfig,
    system: &str,
    user: &str,
    on_delta: &mut dyn FnMut(&str),
    cancel: &Cancel,
) -> Result<AiOutput> {
    let base = if cfg.base_url.trim().is_empty() { DEFAULT_BASE_URL } else { cfg.base_url.trim() };
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));
    let body = json!({
        "model": cfg.model(),
        "stream": true,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user },
        ],
    });
    let resp = crate::http_client(None)?
        .post(url)
        .bearer_auth(cfg.api_key.trim())
        .json(&body)
        .send()
        .map_err(|e| Error::Http(e.to_string()))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().unwrap_or_default();
        return Err(api_error(status.as_u16(), &body));
    }

    let mut text = String::new();
    let mut finish: Option<String> = None;
    let mut done = false;
    read_sse(resp, cancel, |data| {
        if data == "[DONE]" {
            done = true;
            return Ok(false);
        }
        let Ok(ev) = serde_json::from_str::<Value>(data) else { return Ok(true) };
        if let Some(msg) = ev["error"]["message"].as_str() {
            return Err(Error::Api { status: 500, message: msg.to_string() });
        }
        let choice = &ev["choices"][0];
        if let Some(t) = choice["delta"]["content"].as_str() {
            text.push_str(t);
            on_delta(t);
        }
        if let Some(f) = choice["finish_reason"].as_str() {
            finish = Some(f.to_string());
        }
        Ok(true)
    })?;

    match finish.as_deref() {
        Some("content_filter") => Err(Error::Refused("콘텐츠 정책에 따라 거절되었습니다".into())),
        Some("length") => Ok(AiOutput { text, truncated: true }),
        Some(_) => Ok(AiOutput { text, truncated: false }),
        None if done && !text.is_empty() => Ok(AiOutput { text, truncated: false }),
        None => Err(Error::Http("응답이 완료되기 전에 연결이 끊겼습니다".into())),
    }
}
