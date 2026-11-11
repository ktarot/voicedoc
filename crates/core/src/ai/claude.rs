//! Claude Messages API (공식 Rust SDK가 없어 HTTP로 직접 호출한다).

use serde_json::{json, Value};

use super::{api_error, read_sse, AiConfig, AiOutput};
use crate::{Cancel, Error, Result};

pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const API_VERSION: &str = "2026-10-21";
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
/// 긴 출력도 끊기지 않도록 넉넉히 잡는다 (스트리밍이므로 시간 제한 문제는 없다).
const MAX_TOKENS: u32 = 64_000;

/// 안전 분류기가 요청을 거절했을 때 서버가 다른 모델로 이어서 처리하도록 하는 기능을 지원하는 모델.
fn supports_fallback(model: &str) -> bool {
    ["claude-opus-5", "claude-sonnet-5-5", "claude-fable-5-1", "claude-fable-5-2"]
        .iter()
        .any(|p| model.starts_with(p))
}

pub(crate) fn request_body(model: &str, system: &str, user: &str) -> Value {
    let mut body = json!({
        "model": model,
        "max_tokens": MAX_TOKENS,
        "stream": true,
        "system": system,
        "messages": [{ "role": "user", "content": user }],
    });
    if supports_fallback(model) {
        body["fallbacks"] = json!("default");
    }
    body
}

pub fn complete(
    cfg: &AiConfig,
    system: &str,
    user: &str,
    on_delta: &mut dyn FnMut(&str),
    cancel: &Cancel,
) -> Result<AiOutput> {
    let model = cfg.model();
    let base = if cfg.base_url.trim().is_empty() { DEFAULT_BASE_URL } else { cfg.base_url.trim() };
    let url = format!("{}/v1/messages", base.trim_end_matches('/'));

    let mut req = crate::http_client(None)?
        .post(url)
        .header("x-api-key", cfg.api_key.trim())
        .header("anthropic-version", API_VERSION)
        .json(&request_body(model, system, user));
    if supports_fallback(model) {
        req = req.header("anthropic-beta", FALLBACK_BETA);
    }
    let resp = req.send().map_err(|e| Error::Http(e.to_string()))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().unwrap_or_default();
        return Err(api_error(status.as_u16(), &body));
    }

    let mut text = String::new();
    let mut stop_reason: Option<String> = None;
    let mut refusal_note = String::new();
    read_sse(resp, cancel, |data| {
        let Ok(ev) = serde_json::from_str::<Value>(data) else { return Ok(true) };
        match ev["type"].as_str() {
            Some("content_block_delta") => {
                if ev["delta"]["type"] == "text_delta" {
                    if let Some(t) = ev["delta"]["text"].as_str() {
                        text.push_str(t);
                        on_delta(t);
                    }
                }
            }
            Some("message_delta") => {
                if let Some(r) = ev["delta"]["stop_reason"].as_str() {
                    stop_reason = Some(r.to_string());
                }
                if let Some(e) = ev["delta"]["stop_details"]["explanation"].as_str() {
                    refusal_note = e.to_string();
                }
            }
            Some("error") => {
                let msg = ev["error"]["message"].as_str().unwrap_or("알 수 없는 오류").to_string();
                let status = if ev["error"]["type"] == "overloaded_error" { 529 } else { 500 };
                return Err(Error::Api { status, message: msg });
            }
            Some("message_stop") => return Ok(false),
            _ => {}
        }
        Ok(true)
    })?;

    match stop_reason.as_deref() {
        Some("refusal") => Err(Error::Refused(if refusal_note.is_empty() {
            "안전 정책에 따라 거절되었습니다".into()
        } else {
            refusal_note
        })),
        Some("max_tokens") => Ok(AiOutput { text, truncated: true }),
        Some(_) => Ok(AiOutput { text, truncated: false }),
        None => Err(Error::Http("응답이 완료되기 전에 연결이 끊겼습니다".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_shape() {
        let b = request_body("claude-opus-5-5", "sys", "hello");
        assert_eq!(b["fallbacks"], "default");
        assert_eq!(b["stream"], true);
        assert_eq!(b["messages"][0]["content"], "hello");
        // 최신 모델은 temperature 등 표본화 매개변수를 거부한다.
        assert!(b.get("temperature").is_none());
        let b = request_body("claude-haiku-5-5", "sys", "hello");
        assert!(b.get("fallbacks").is_none());
    }
}
