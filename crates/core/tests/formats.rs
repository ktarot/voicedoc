//! 실제 파일로 디코딩과 내보내기를 확인한다.

use std::path::PathBuf;

use voicedoc_core::doc::Document;
use voicedoc_core::export::{self, Format};
use voicedoc_core::{audio, Cancel};

fn testdata(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata").join(name)
}

/// 두 신호의 유사도(정규화 상관). AAC는 인코더 지연(약 0.02초)이 남으므로
/// ±0.05초 범위에서 가장 잘 맞는 위치를 찾는다.
fn best_correlation(a: &[f32], b: &[f32]) -> f32 {
    const MAX_SHIFT: usize = 800;
    let n = a.len().min(b.len()) - MAX_SHIFT;
    let corr = |x: &[f32], y: &[f32]| {
        let (mut dot, mut xx, mut yy) = (0.0f64, 0.0f64, 0.0f64);
        for (p, q) in x.iter().zip(y.iter()).step_by(3) {
            dot += (p * q) as f64;
            xx += (p * p) as f64;
            yy += (q * q) as f64;
        }
        (dot / (xx * yy).sqrt()) as f32
    };
    (0..MAX_SHIFT)
        .flat_map(|s| [corr(&a[s..s + n], &b[..n]), corr(&a[..n], &b[s..s + n])])
        .fold(0.0, f32::max)
}

#[test]
fn decodes_every_supported_format() {
    let cancel = Cancel::new();
    let reference = audio::decode_file(&testdata("jfk.wav"), |_| {}, &cancel).unwrap();
    assert_eq!(reference.len(), 176_000, "11초 분량의 16kHz 샘플");

    for name in ["jfk.m4a", "jfk.mp3", "jfk.mp4", "jfk.flac", "jfk.ogg"] {
        let mut last_progress = 0.0;
        let samples = audio::decode_file(&testdata(name), |p| last_progress = p, &cancel)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let secs = samples.len() as f32 / 16_000.0;
        assert!((secs - 11.0).abs() < 0.15, "{name}: 길이 {secs}초");
        assert_eq!(last_progress, 1.0, "{name}: 진행률");
        let corr = best_correlation(&reference, &samples);
        assert!(corr > 0.97, "{name}: 원본과의 유사도 {corr}");
    }
}

#[test]
fn rejects_non_audio_and_honours_cancel() {
    let dir = tempfile::tempdir().unwrap();
    let bogus = dir.path().join("not-audio.mp3");
    std::fs::write(&bogus, b"this is not audio at all").unwrap();
    assert!(audio::decode_file(&bogus, |_| {}, &Cancel::new()).is_err());

    let cancel = Cancel::new();
    cancel.cancel();
    let err = audio::decode_file(&testdata("jfk.mp3"), |_| {}, &cancel).unwrap_err();
    assert!(matches!(err, voicedoc_core::Error::Cancelled));
}

#[test]
fn wav_roundtrip() {
    let cancel = Cancel::new();
    let reference = audio::decode_file(&testdata("jfk.wav"), |_| {}, &cancel).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.wav");
    std::fs::write(&path, audio::encode_wav(&reference)).unwrap();
    let again = audio::decode_file(&path, |_| {}, &cancel).unwrap();
    assert_eq!(again.len(), reference.len());
    assert!(best_correlation(&reference, &again) > 0.999);
}

#[test]
fn exports_all_formats() {
    let body = "## 개요\n신제품 출시 일정을 논의했다. **3월 출시**로 확정.\n\n## 할 일\n- 김 팀장: 일정표 공유\n- 디자인 시안 검토 😀\n\n"
        .to_string()
        + &"회의 내용을 길게 적어 여러 쪽이 되는지 확인합니다. ".repeat(400);
    let doc = Document::new("주간 회의", body);
    let dir = tempfile::tempdir().unwrap();

    for format in Format::ALL {
        let path = export::unique_path(dir.path(), "주간 회의", format);
        export::save(&doc, format, &path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        match format {
            Format::Pdf => assert!(bytes.starts_with(b"%PDF-")),
            Format::Docx => assert!(bytes.starts_with(b"PK")),
            Format::Md => assert!(String::from_utf8(bytes).unwrap().starts_with("# 주간 회의\n")),
            Format::Txt => assert!(String::from_utf8(bytes).unwrap().contains("- 김 팀장: 일정표 공유\r\n")),
        }
    }
    // 같은 이름이 이미 있으면 덮어쓰지 않고 번호를 붙인다.
    let second = export::unique_path(dir.path(), "주간 회의", Format::Pdf);
    assert!(second.ends_with("주간 회의 (2).pdf"));
}
