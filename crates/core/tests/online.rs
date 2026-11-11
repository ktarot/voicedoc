//! 가짜 서버로 온라인 기능의 요청 형식과 응답 처리를 확인한다.

use std::sync::mpsc;
use std::thread;

use voicedoc_core::ai::{self, AiConfig, Provider, Task};
use voicedoc_core::stt::{cloud, SttOptions};
use voicedoc_core::{Cancel, Error};

struct Seen {
    url: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
    fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap()
    }
}

/// 요청마다 준비된 응답(상태 코드, 본문)을 차례로 돌려주는 서버.
fn serve(responses: Vec<(u16, String)>) -> (String, mpsc::Receiver<Seen>) {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let base = format!("http://{}", server.server_addr());
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for (status, body) in responses {
            let Ok(mut req) = server.recv() else { return };
            let mut data = Vec::new();
            req.as_reader().read_to_end(&mut data).unwrap();
            let headers = req
                .headers()
                .iter()
                .map(|h| (h.field.to_string(), h.value.to_string()))
                .collect();
            tx.send(Seen { url: req.url().to_string(), headers, body: data }).unwrap();
            req.respond(tiny_http::Response::from_string(body).with_status_code(status)).unwrap();
        }
    });
    (base, rx)
}

fn claude_stream(text_parts: &[&str], stop_reason: &str) -> String {
    let mut s = String::from(
        "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\"}}\n\n\
         event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\"}}\n\n\
         event: ping\ndata: {\"type\":\"ping\"}\n\n",
    );
    for part in text_parts {
        let delta = serde_json::json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":part}});
        s.push_str(&format!("event: content_block_delta\ndata: {delta}\n\n"));
    }
    s.push_str(&format!(
        "event: message_delta\ndata: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"{stop_reason}\"}}}}\n\n\
         event: message_stop\ndata: {{\"type\":\"message_stop\"}}\n\n"
    ));
    s
}

fn config(provider: Provider, base_url: &str) -> AiConfig {
    AiConfig { provider, api_key: "test-key".into(), model: String::new(), base_url: base_url.into() }
}

#[test]
fn claude_summary_streams_text() {
    let (base, rx) = serve(vec![(200, claude_stream(&["# 요약\n", "핵심 내용"], "end_turn"))]);
    let mut streamed = String::new();
    let out = ai::run_task(
        &config(Provider::Claude, &base),
        &Task::Summary,
        "안녕하세요. 회의를 시작합니다.",
        &mut |d| streamed.push_str(d),
        &Cancel::new(),
    )
    .unwrap();
    assert_eq!(out.text, "# 요약\n핵심 내용");
    assert_eq!(streamed, out.text);
    assert!(!out.truncated);

    let seen = rx.recv().unwrap();
    assert_eq!(seen.url, "/v1/messages");
    assert_eq!(seen.header("x-api-key"), Some("test-key"));
    assert_eq!(seen.header("anthropic-version"), Some("2026-10-21"));
    assert_eq!(seen.header("anthropic-beta"), Some("server-side-fallback-2026-07-01"));
    let body = seen.json();
    assert_eq!(body["model"], "claude-opus-5-5");
    assert_eq!(body["messages"][0]["content"], "안녕하세요. 회의를 시작합니다.");
    assert!(body["system"].as_str().unwrap().contains("요약"));
}

#[test]
fn claude_refusal_truncation_and_errors() {
    let (base, _rx) = serve(vec![
        (200, claude_stream(&[], "refusal")),
        (200, claude_stream(&["잘린 글"], "max_tokens")),
        (401, r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#.into()),
        (200, "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"중간\"}}\n\n".into()),
    ]);
    let cfg = config(Provider::Claude, &base);
    let run = || ai::run_task(&cfg, &Task::Minutes, "내용", &mut |_| {}, &Cancel::new());

    assert!(matches!(run().unwrap_err(), Error::Refused(_)));
    let out = run().unwrap();
    assert!(out.truncated && out.text == "잘린 글");
    match run().unwrap_err() {
        Error::Api { status: 401, message } => assert!(message.contains("API 키")),
        other => panic!("예상과 다른 오류: {other}"),
    }
    // 끝맺음 없이 끊긴 응답은 성공으로 치지 않는다.
    assert!(matches!(run().unwrap_err(), Error::Http(_)));
}

#[test]
fn openai_translation_is_chunked_and_joined() {
    let chunk = |text: &str| {
        format!(
            "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            serde_json::json!({"choices":[{"delta":{"content":text}}]}),
            serde_json::json!({"choices":[{"delta":{},"finish_reason":"stop"}]}),
        )
    };
    let (base, rx) = serve(vec![(200, chunk("First part.")), (200, chunk("Second part."))]);
    // 8,000자 제한을 넘도록 두 문단을 만든다.
    let text = format!("{}\n\n{}", "가".repeat(5000), "나".repeat(5000));
    let out = ai::run_task(
        &config(Provider::OpenAi, &base),
        &Task::Translate { target: "영어".into() },
        &text,
        &mut |_| {},
        &Cancel::new(),
    )
    .unwrap();
    assert_eq!(out.text, "First part.\n\nSecond part.");

    let first = rx.recv().unwrap();
    assert_eq!(first.url, "/chat/completions");
    assert_eq!(first.header("authorization"), Some("Bearer test-key"));
    let body = first.json();
    assert_eq!(body["model"], "gpt-5");
    assert!(body["messages"][0]["content"].as_str().unwrap().contains("영어"));
    assert_eq!(body["messages"][1]["content"].as_str().unwrap().chars().count(), 5000);
    let second = rx.recv().unwrap();
    assert!(second.json()["messages"][1]["content"].as_str().unwrap().starts_with('나'));
}

#[test]
fn missing_key_is_reported_before_any_request() {
    let mut cfg = config(Provider::Claude, "http://127.0.0.1:9");
    cfg.api_key = "  ".into();
    let err = ai::run_task(&cfg, &Task::Summary, "내용", &mut |_| {}, &Cancel::new()).unwrap_err();
    assert!(matches!(err, Error::Config(_)));
}

#[test]
fn cloud_transcription_offsets_chunks() {
    let reply = |text: &str| {
        serde_json::json!({
            "language": "korean",
            "text": text,
            "segments": [{"start": 1.0, "end": 2.5, "text": format!(" {text}")}],
        })
        .to_string()
    };
    let (base, rx) = serve(vec![(200, reply("첫 조각")), (200, reply("둘째 조각"))]);
    // 11분 분량 → 두 조각
    let samples = vec![0.01f32; 16_000 * 660];
    let cfg = cloud::CloudSttConfig {
        api_key: "test-key".into(),
        model: cloud::DEFAULT_MODEL.into(),
        base_url: base,
    };
    let opts = SttOptions { language: Some("ko".into()), prompt: None };
    let mut progress = Vec::new();
    let t = cloud::transcribe(&cfg, &samples, &opts, |p| progress.push(p), &Cancel::new()).unwrap();

    assert_eq!(t.segments.len(), 2);
    assert_eq!(t.segments[0].text, "첫 조각");
    assert_eq!(t.segments[0].start_ms, 1000);
    // 둘째 조각의 시각은 첫 조각 길이(약 10분)만큼 밀려야 한다.
    assert!(t.segments[1].start_ms > 585_000 && t.segments[1].start_ms <= 601_000);
    assert_eq!(progress, vec![0, 50, 100]);

    let seen = rx.recv().unwrap();
    assert_eq!(seen.url, "/audio/transcriptions");
    assert!(seen.header("content-type").unwrap().starts_with("multipart/form-data"));
    let body = String::from_utf8_lossy(&seen.body);
    assert!(body.contains("name=\"model\"") && body.contains("whisper-1"));
    assert!(body.contains("verbose_json") && body.contains("name=\"language\""));
    assert!(seen.body.len() < 25 * 1024 * 1024, "업로드 한도 이내");
}
