//! 실제 모델로 인식과 취소를 확인한다. 모델 파일(models/ggml-base.bin)이 없으면 건너뛴다.
#![cfg(feature = "local-stt")]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use voicedoc_core::stt::{local, SttOptions};
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
    let opts = SttOptions { language: Some("en".into()), prompt: None };
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
