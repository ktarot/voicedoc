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

    let t = stt::transcribe_speech(&audio, &opts, &cancel, &|_| {}, |speech| {
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
    let t = stt::transcribe_speech(&audio, &SttOptions::default(), &cancel, &|_| {}, |_| {
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
    let t = stt::transcribe_speech(&audio, &opts, &cancel, &|_| {}, |speech| {
        seen = speech.len();
        Ok(voicedoc_core::Transcript::default())
    })
    .unwrap();
    assert_eq!(seen, audio.len());
    assert_eq!(t.duration_ms, 5_000);
}
