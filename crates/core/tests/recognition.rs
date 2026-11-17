//! 실제 모델로 인식과 취소를 확인한다. 모델 파일(models/ggml-base.bin)이 없으면 건너뛴다.
#![cfg(feature = "local-stt")]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use voicedoc_core::stt::{self, local, SttOptions};
use voicedoc_core::{audio, Cancel, Error};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn model() -> Option<PathBuf> {
    let path = root().join("models/ggml-base.bin");
    if !path.is_file() {
        eprintln!("모델이 없어 건너뜁니다: {}", path.display());
        return None;
    }
    Some(path)
}

#[test]
fn recognises_speech_from_m4a() {
    let Some(model) = model() else { return };
    let cancel = Cancel::new();
    let samples = audio::decode_file(&root().join("testdata/jfk.m4a"), |_| {}, &cancel).unwrap();
    let t = local::transcribe(&model, &samples, &SttOptions::default(), |_| {}, &cancel).unwrap();
    let text = t.to_text(false).to_lowercase();
    assert!(text.contains("ask not what your country can do for you"), "인식 결과: {text}");
    assert_eq!(t.language.as_deref(), Some("en"));
    assert!((11_000..11_100).contains(&t.duration_ms), "길이: {}ms", t.duration_ms);
}

#[test]
fn cancel_stops_recognition_early() {
    let Some(model) = model() else { return };
    let cancel = Cancel::new();
    let clip = audio::decode_file(&root().join("testdata/jfk.wav"), |_| {}, &cancel).unwrap();
    // 약 9분 분량: 끝까지 인식하면 한참 걸린다.
    let long: Vec<f32> = clip.iter().copied().cycle().take(clip.len() * 50).collect();

    let stopper = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1500));
        stopper.cancel();
    });
    let started = Instant::now();
    let opts = SttOptions { language: Some("en".into()), prompt: None, ..Default::default() };
    let err = local::transcribe(&model, &long, &opts, |_| {}, &cancel).unwrap_err();
    assert!(matches!(err, Error::Cancelled), "예상과 다른 결과: {err}");
    assert!(started.elapsed() < Duration::from_secs(30), "취소가 너무 늦습니다: {:?}", started.elapsed());
}

#[test]
fn missing_model_is_a_clear_error() {
    let err = local::transcribe(
        &root().join("models/없는-모델.bin"),
        &[0.0; 16_000],
        &SttOptions::default(),
        |_| {},
        &Cancel::new(),
    )
    .unwrap_err();
    assert!(matches!(err, Error::Model(_)));
}

/// 앞뒤와 중간에 긴 침묵이 있어도 말만 받아 적고, 시각은 원래 위치를 가리켜야 한다.
#[test]
fn skips_silence_and_keeps_original_times() {
    let Some(model) = model() else { return };
    let cancel = Cancel::new();
    let clip = audio::decode_file(&root().join("testdata/jfk.wav"), |_| {}, &cancel).unwrap();
    let silence = vec![0.0f32; 16_000 * 40];
    // 침묵 40초 + 말 11초 + 침묵 40초 + 말 11초 + 침묵 40초
    let audio: Vec<f32> = [&silence[..], &clip, &silence, &clip, &silence].concat();
    let opts = SttOptions { language: Some("en".into()), ..Default::default() };

    let t = stt::transcribe_speech(&audio, &opts, &cancel, &|_| {}, |speech, _| {
        // 엔진에는 말소리 구간만 넘어가야 한다 (전체 142초 중 약 22초).
        assert!(speech.len() < 16_000 * 30, "엔진에 넘어간 길이: {}초", speech.len() / 16_000);
        local::transcribe(&model, speech, &opts, |_| {}, &cancel)
    })
    .unwrap();

    let paragraphs = t.paragraphs();
    assert_eq!(paragraphs.len(), 2, "{paragraphs:?}");
    for (start, text) in &paragraphs {
        assert!(text.to_lowercase().contains("ask not what your country can do for you"), "{text}");
        assert_eq!(text.to_lowercase().matches("fellow americans").count(), 1, "되풀이 없음: {text}");
        let _ = start;
    }
    assert!((39_000..42_500).contains(&paragraphs[0].0), "첫 문단 시각: {}", paragraphs[0].0);
    assert!((90_000..93_500).contains(&paragraphs[1].0), "둘째 문단 시각: {}", paragraphs[1].0);
    assert_eq!(t.duration_ms, 142_000);
}

#[test]
fn silence_only_gives_no_text() {
    let cancel = Cancel::new();
    let audio = vec![0.0f32; 16_000 * 60];
    let t = stt::transcribe_speech(&audio, &SttOptions::default(), &cancel, &|_| {}, |_, _| {
        panic!("말소리가 없으면 엔진을 부르지 않는다")
    })
    .unwrap();
    assert!(t.segments.is_empty());
    assert_eq!(t.duration_ms, 60_000);
}

/// 건너뛰기를 끄면 음성 전체가 그대로 엔진에 넘어간다.
#[test]
fn keep_silence_passes_audio_through() {
    let cancel = Cancel::new();
    let audio = vec![0.0f32; 16_000 * 5];
    let opts = SttOptions { keep_silence: true, ..Default::default() };
    let mut seen = 0;
    let t = stt::transcribe_speech(&audio, &opts, &cancel, &|_| {}, |speech, _| {
        seen = speech.len();
        Ok(voicedoc_core::Transcript::default())
    })
    .unwrap();
    assert_eq!(seen, audio.len());
    assert_eq!(t.duration_ms, 5_000);
}

/// 긴 음성은 약 10분씩 나뉘어 엔진에 넘어가고, 각 묶음의 시각은 원래 위치로 돌아와야 한다.
/// 한 묶음이 헛돌아도 다음 묶음은 영향을 받지 않는다.
#[test]
fn long_audio_goes_to_the_engine_in_blocks() {
    let cancel = Cancel::new();
    let audio = vec![0.0f32; 16_000 * 60 * 25];
    let opts = SttOptions { keep_silence: true, language: Some("ko".into()), ..Default::default() };
    let mut spans = Vec::new();
    let t = stt::transcribe_speech(&audio, &opts, &cancel, &|_| {}, |block, span| {
        assert!(block.len() <= 16_000 * 600, "묶음 길이: {}초", block.len() / 16_000);
        spans.push(span);
        let n = spans.len();
        // 둘째 묶음은 같은 말만 되풀이하는 상황을 흉내 낸다.
        let text = ["첫째 묶음에서 받아 적은 문장입니다.", "같은 말만 되풀이합니다.", "셋째 묶음에서 받아 적은 문장입니다."][n - 1];
        let segments = (0..3)
            .map(|i| voicedoc_core::Segment { start_ms: 1000 + i * 2000, end_ms: 3000 + i * 2000, text: text.into() })
            .collect();
        Ok(voicedoc_core::Transcript { segments, ..Default::default() })
    })
    .unwrap();

    assert_eq!(spans.len(), 3);
    assert_eq!(spans[0].start, 0.0);
    assert_eq!(spans[2].end, 1.0);
    assert!(spans.windows(2).all(|w| w[0].end == w[1].start));

    // 묶음마다 되풀이는 하나로 줄고, 셋째 묶음의 글은 그대로 남는다.
    let starts: Vec<u64> = t.segments.iter().map(|s| s.start_ms / 1000).collect();
    assert_eq!(t.segments.len(), 3, "{:?}", t.segments);
    assert_eq!(starts[0], 1);
    // 묶음 경계는 10분 근처의 가장 조용한 곳(최대 15초 앞)에 놓인다.
    assert!((586..=601).contains(&starts[1]), "둘째 묶음 시각: {}", starts[1]);
    assert!((1171..=1201).contains(&starts[2]), "셋째 묶음 시각: {}", starts[2]);
    assert!(t.segments[2].text.starts_with("셋째 묶음"));
    assert_eq!(t.duration_ms, 25 * 60_000);
}
