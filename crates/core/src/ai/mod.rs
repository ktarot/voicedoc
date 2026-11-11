//! 온라인 AI 후처리: 요약, 회의록 정리, 번역. Claude와 OpenAI 중 선택.

pub mod claude;
pub mod openai;

use std::io::{BufRead, BufReader, Read};

use serde::{Deserialize, Serialize};

use crate::{Cancel, Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Claude,
    OpenAi,
}

impl Provider {
    pub fn default_model(self) -> &'static str {
        match self {
            Provider::Claude => claude::DEFAULT_MODEL,
            Provider::OpenAi => openai::DEFAULT_MODEL,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AiConfig {
    pub provider: Provider,
    pub api_key: String,
    /// 비어 있으면 제공자 기본 모델.
    pub model: String,
    /// 비어 있으면 제공자 기본 주소.
    pub base_url: String,
}

impl AiConfig {
    fn model(&self) -> &str {
        if self.model.trim().is_empty() {
            self.provider.default_model()
        } else {
            self.model.trim()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Task {
    /// 핵심 요약
    Summary,
    /// 회의록 형식 정리
    Minutes,
    /// 번역. `target`은 "영어", "한국어" 같은 언어 이름.
    Translate { target: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct AiOutput {
    pub text: String,
    /// 출력 길이 제한에 걸려 끝이 잘렸는지 여부.
    pub truncated: bool,
}

/// 이 길이를 넘는 녹취는 나눠서 요약한 뒤 합친다.
const SUMMARY_CHUNK_CHARS: usize = 200_000;
/// 번역은 출력 길이가 입력만큼 길어지므로 작게 나눈다.
const TRANSLATE_CHUNK_CHARS: usize = 8_000;

const FORMAT_RULES: &str = "출력 형식: 마크다운의 제목(#, ##), 글머리표(-), 굵게(**)만 사용하고 표와 코드 블록은 쓰지 않는다. \
인사말이나 설명 없이 결과 문서만 출력한다.";

fn system_prompt(task: &Task) -> String {
    match task {
        Task::Summary => format!(
            "당신은 음성 녹취를 정리하는 편집자다. 사용자가 주는 녹취(음성 인식 결과)를 읽고 핵심 내용을 요약한다.\n\
             - 녹취와 같은 언어로 쓴다.\n\
             - 맨 위에 '# 요약' 제목을 두고, 전체를 두세 문장으로 요약한 뒤 '## 주요 내용'에 요점을 글머리표로 정리한다.\n\
             - 음성 인식 오류로 보이는 단어는 문맥에 맞게 바로잡되, 녹취에 없는 내용을 지어내지 않는다.\n{FORMAT_RULES}"
        ),
        Task::Minutes => format!(
            "당신은 회의록 작성자다. 사용자가 주는 녹취(음성 인식 결과)를 회의록으로 정리한다.\n\
             - 녹취와 같은 언어로 쓴다.\n\
             - 구성: '# 회의록', '## 개요', '## 논의 내용'(주제별 글머리표), '## 결정 사항', '## 할 일'(담당자·기한이 언급된 경우 함께 적는다).\n\
             - 해당 내용이 없는 항목은 '없음'이라고 적는다. 녹취에 없는 내용을 지어내지 않는다.\n\
             - 음성 인식 오류로 보이는 단어는 문맥에 맞게 바로잡는다.\n{FORMAT_RULES}"
        ),
        Task::Translate { target } => format!(
            "당신은 전문 번역가다. 사용자가 주는 글을 {target}(으)로 번역한다.\n\
             - 문단 구분, 제목 기호(#), 글머리표, [00:00:00] 형식의 시각 표시는 그대로 유지한다.\n\
             - 내용을 줄이거나 덧붙이지 않고 전부 번역한다.\n\
             - 설명 없이 번역문만 출력한다."
        ),
    }
}

/// 문단 경계를 지키면서 `limit` 글자 이하의 조각으로 나눈다.
pub fn split_text(text: &str, limit: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut cur = String::new();
    let mut cur_len = 0;
    for para in text.split("\n\n") {
        let len = para.chars().count();
        if cur_len > 0 && cur_len + len + 2 > limit {
            chunks.push(std::mem::take(&mut cur));
            cur_len = 0;
        }
        if len > limit {
            // 문단 하나가 너무 길면 글자 수로 자른다.
            let chars: Vec<char> = para.chars().collect();
            for piece in chars.chunks(limit) {
                chunks.push(piece.iter().collect());
            }
            continue;
        }
        if cur_len > 0 {
            cur.push_str("\n\n");
            cur_len += 2;
        }
        cur.push_str(para);
        cur_len += len;
    }
    if !cur.trim().is_empty() {
        chunks.push(cur);
    }
    chunks
}

fn complete(
    cfg: &AiConfig,
    system: &str,
    user: &str,
    on_delta: &mut dyn FnMut(&str),
    cancel: &Cancel,
) -> Result<AiOutput> {
    match cfg.provider {
        Provider::Claude => claude::complete(cfg, system, user, on_delta, cancel),
        Provider::OpenAi => openai::complete(cfg, system, user, on_delta, cancel),
    }
}

/// 녹취 텍스트에 AI 작업을 수행한다. `on_delta`로 생성 중인 글 조각이 전달된다.
pub fn run_task(
    cfg: &AiConfig,
    task: &Task,
    text: &str,
    on_delta: &mut dyn FnMut(&str),
    cancel: &Cancel,
) -> Result<AiOutput> {
    if cfg.api_key.trim().is_empty() {
        return Err(Error::Config("AI 기능을 쓰려면 설정에서 API 키를 입력해 주세요".into()));
    }
    if text.trim().is_empty() {
        return Err(Error::Config("처리할 내용이 없습니다".into()));
    }
    let system = system_prompt(task);
    match task {
        Task::Translate { .. } => {
            let mut out = String::new();
            let mut truncated = false;
            for (i, chunk) in split_text(text, TRANSLATE_CHUNK_CHARS).iter().enumerate() {
                cancel.check()?;
                if i > 0 {
                    out.push_str("\n\n");
                    on_delta("\n\n");
                }
                let r = complete(cfg, &system, chunk, on_delta, cancel)?;
                out.push_str(r.text.trim());
                truncated |= r.truncated;
            }
            Ok(AiOutput { text: out, truncated })
        }
        Task::Summary | Task::Minutes => {
            let chunks = split_text(text, SUMMARY_CHUNK_CHARS);
            if chunks.len() <= 1 {
                return complete(cfg, &system, text, on_delta, cancel);
            }
            // 아주 긴 녹취: 부분별로 요점을 뽑은 뒤 한 번에 정리한다.
            let part_system = "당신은 음성 녹취를 정리하는 편집자다. 사용자가 주는 녹취 일부에서 \
                나중에 전체 문서를 만들 수 있도록 중요한 내용, 결정, 할 일, 수치를 빠짐없이 글머리표로 정리한다. \
                녹취와 같은 언어로 쓰고, 녹취에 없는 내용은 쓰지 않는다.";
            let mut notes = String::new();
            for (i, chunk) in chunks.iter().enumerate() {
                cancel.check()?;
                let r = complete(cfg, part_system, chunk, &mut |_| {}, cancel)?;
                notes.push_str(&format!("[녹취 {}/{} 부분 정리]\n{}\n\n", i + 1, chunks.len(), r.text.trim()));
            }
            complete(cfg, &system, &notes, on_delta, cancel)
        }
    }
}

/// 서버 오류 응답에서 사람이 읽을 메시지를 꺼낸다.
pub(crate) fn api_error(status: u16, body: &str) -> Error {
    let message = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
        .unwrap_or_else(|| body.chars().take(300).collect());
    let hint = match status {
        401 | 403 => " (API 키를 확인해 주세요)",
        404 => " (모델 이름을 확인해 주세요)",
        429 => " (사용량 한도에 걸렸습니다. 잠시 후 다시 시도해 주세요)",
        _ => "",
    };
    Error::Api { status, message: format!("{message}{hint}") }
}

/// SSE 응답에서 `data:` 줄을 하나씩 꺼내 처리한다. 처리 함수가 false를 돌려주면 멈춘다.
pub(crate) fn read_sse(
    reader: impl Read,
    cancel: &Cancel,
    mut on_data: impl FnMut(&str) -> Result<bool>,
) -> Result<()> {
    for line in BufReader::new(reader).lines() {
        cancel.check()?;
        let line = line.map_err(|e| Error::Http(format!("응답 수신 중 연결이 끊겼습니다: {e}")))?;
        if let Some(data) = line.strip_prefix("data:") {
            if !on_data(data.trim())? {
                break;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_paragraphs() {
        let text = format!("{}\n\n{}\n\n{}", "가".repeat(50), "나".repeat(50), "다".repeat(50));
        let chunks = split_text(&text, 110);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks.join("\n\n"), text);
    }

    #[test]
    fn splits_oversized_paragraph() {
        let chunks = split_text(&"가".repeat(250), 100);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks.concat().chars().count(), 250);
    }

    #[test]
    fn error_message_extraction() {
        let e = api_error(401, r#"{"error":{"type":"authentication_error","message":"invalid x-api-key"}}"#);
        assert!(e.to_string().contains("invalid x-api-key"));
        assert!(e.to_string().contains("API 키"));
    }
}
