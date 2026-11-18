pub mod clean;
pub mod cloud;
#[cfg(feature = "local-stt")]
pub mod local;
pub mod vad;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SttOptions {
    /// 언어 코드 ("ko", "en" …). None이면 자동 감지.
    pub language: Option<String>,
    /// 고유명사·전문용어 등 인식에 참고할 힌트 문장.
    pub prompt: Option<String>,
    /// 그래픽카드가 있어도 CPU로만 인식한다 (문제 해결용).
    #[serde(default)]
    pub cpu_only: bool,
    /// 침묵·잡음 구간을 건너뛰지 않고 음성 전체를 그대로 인식한다.
    /// 기본값(false)은 말소리 구간만 골라 인식한다.
    #[serde(default)]
    pub keep_silence: bool,
}

/// 인식 엔진이 이 PC의 CPU에서 돌 수 있는지 확인한다.
///
/// 엔진은 AVX2·FMA·F16C 명령어를 쓰도록 빌드된다. 이 명령어가 없는 CPU(2013년 이전 인텔,
/// 2017년 이전 AMD 등)에서 그대로 돌리면 오류 메시지 없이 프로그램이 꺼지므로 미리 막는다.
pub fn check_cpu() -> crate::Result<()> {
    #[cfg(target_arch = "x86_64")]
    {
        let missing: Vec<&str> = [
            ("AVX2", std::arch::is_x86_feature_detected!("avx2")),
            ("FMA", std::arch::is_x86_feature_detected!("fma")),
            ("F16C", std::arch::is_x86_feature_detected!("f16c")),
        ]
        .into_iter()
        .filter(|(_, ok)| !ok)
        .map(|(name, _)| name)
        .collect();
        if !missing.is_empty() {
            return Err(crate::Error::Stt(format!(
                "이 PC의 CPU는 음성 인식에 필요한 명령어({})를 지원하지 않습니다. 설정에서 클라우드 인식을 사용해 주세요",
                missing.join(", ")
            )));
        }
    }
    Ok(())
}

/// 인식 엔진이 빌드될 때 켜진 CPU 명령어 등 (문제 해결용).
#[cfg(feature = "local-stt")]
pub fn engine_info() -> String {
    whisper_rs::print_system_info().trim().to_string()
}

/// whisper.cpp가 표준 오류로 쏟아내는 기록을 막는다.
#[cfg(feature = "local-stt")]
pub(crate) fn quiet_engine_logs() {
    static HOOK: std::sync::Once = std::sync::Once::new();
    // 문제를 살필 때는 환경 변수 VOICEDOC_ENGINE_LOG를 주면 엔진 기록이 그대로 나온다.
    if std::env::var_os("VOICEDOC_ENGINE_LOG").is_some() {
        return;
    }
    HOOK.call_once(whisper_rs::install_logging_hooks);
}

/// 인식 엔진은 한 번에 하나만 돌린다. 같은 그래픽카드에서 두 인식을 동시에 돌리면 안정적이지 않고,
/// CPU에서도 엔진의 작업 스레드끼리 서로 기다리느라 몇 배로 느려진다.
#[cfg(feature = "local-stt")]
pub(crate) fn engine_lock() -> std::sync::MutexGuard<'static, ()> {
    static RUN: std::sync::Mutex<()> = std::sync::Mutex::new(());
    RUN.lock().unwrap_or_else(|e| e.into_inner())
}

/// 인식 엔진에 넘기는 음성은 최소 이만큼은 되어야 한다.
const MIN_ENGINE_SAMPLES: usize = crate::audio::SAMPLE_RATE as usize * 12 / 10;

/// 전체 작업 중 지금 인식하는 묶음이 차지하는 범위 (0.0~1.0).
/// 엔진이 알려 주는 묶음 안의 진행률을 전체 진행률로 바꿀 때 쓴다.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub start: f32,
    pub end: f32,
}

impl Span {
    /// 묶음 안의 진행률(0~100)을 전체 진행률(0~100)로 바꾼다.
    pub fn overall(&self, percent: i32) -> i32 {
        let p = percent.clamp(0, 100) as f32 / 100.0;
        ((self.start + (self.end - self.start) * p) * 100.0).round() as i32
    }
}

/// 말소리 구간만 골라 `engine`으로 인식하고, 시각을 원래 위치로 되돌린 뒤 결과를 정리한다.
///
/// 로컬·클라우드 인식 모두 이 함수를 거친다. 음성은 약 10분씩의 묶음으로 나뉘어
/// `engine`에 차례로 넘어간다 (16kHz 모노 표본과, 그 묶음이 전체에서 차지하는 범위).
/// `scan_progress`에는 말소리 구간을 찾는 동안 0~100 값이 전달된다.
pub fn transcribe_speech(
    samples: &[f32],
    opts: &SttOptions,
    cancel: &crate::Cancel,
    scan_progress: &(dyn Fn(i32) + Sync),
    mut engine: impl FnMut(&[f32], Span) -> crate::Result<crate::Transcript>,
) -> crate::Result<crate::Transcript> {
    if !opts.keep_silence {
        // 말소리 감지도 같은 엔진(ggml)으로 CPU에서 돈다.
        check_cpu()?;
    }
    let blocks = if opts.keep_silence { None } else { speech_blocks(samples, scan_progress, cancel) };
    cancel.check()?;
    // 말소리 감지를 쓰지 않으면 음성 전체를 10분씩 그대로 넘긴다.
    let blocks = blocks.unwrap_or_else(|| cloud::split_points(samples).into_iter().map(|r| vec![r]).collect());

    let total: usize = blocks.iter().flatten().map(|(s, e)| e - s).sum();
    let mut language = opts.language.clone();
    let mut segments = Vec::new();
    let mut done = 0usize;
    for regions in &blocks {
        cancel.check()?;
        let mut packed = vad::Packed::new(samples, regions);
        if packed.is_empty() {
            continue;
        }
        let len: usize = regions.iter().map(|(s, e)| e - s).sum();
        let span = Span { start: done as f32 / total as f32, end: (done + len) as f32 / total as f32 };
        done += len;
        if packed.samples.len() < MIN_ENGINE_SAMPLES {
            packed.samples.resize(MIN_ENGINE_SAMPLES, 0.0);
        }
        let part = engine(&packed.samples, span)?;
        if language.is_none() {
            language = part.language;
        }
        segments.extend(part.segments.into_iter().map(|mut seg| {
            let (start, end) = (seg.start_ms, seg.end_ms);
            seg.start_ms = packed.start_to_original_ms(start);
            seg.end_ms = packed.end_to_original_ms(end).max(seg.start_ms);
            seg
        }));
    }
    let segments = clean::clean(segments, opts.prompt.as_deref());
    let speech: Vec<(u64, u64)> = if opts.keep_silence {
        Vec::new()
    } else {
        let ms = |x: usize| crate::audio::duration_ms(x);
        blocks.iter().flatten().map(|&(s, e)| (ms(s), ms(e))).collect()
    };
    Ok(crate::Transcript {
        language,
        duration_ms: crate::audio::duration_ms(samples.len()),
        unrecognized: unrecognized_speech(&speech, &segments),
        segments,
    })
}

/// 말소리가 이만큼 이어졌는데 글이 하나도 없으면 받아 적지 못한 것으로 본다 (1분).
const UNRECOGNIZED_MIN_MS: u64 = 60_000;

/// 말소리 구간 가운데 받아 적은 글이 전혀 없는 긴 부분을 찾는다.
///
/// 인식 엔진이 헛돌아 같은 말만 되풀이하면 정리 단계에서 지워져 그 부분이 통째로 빈다.
/// 조용히 넘어가면 사용자는 글이 빠진 줄 모르므로, 빈 부분을 찾아 알려 준다.
pub fn unrecognized_speech(speech: &[(u64, u64)], segments: &[crate::Segment]) -> Vec<(u64, u64)> {
    let mut out: Vec<(u64, u64)> = Vec::new();
    // 글이 없는 틈: 처음~첫 구간, 구간 사이, 마지막 구간~끝
    let mut gaps: Vec<(u64, u64)> = Vec::new();
    let mut cursor = 0u64;
    for seg in segments {
        if seg.start_ms > cursor {
            gaps.push((cursor, seg.start_ms));
        }
        cursor = cursor.max(seg.end_ms);
    }
    gaps.push((cursor, u64::MAX));

    for (gap_start, gap_end) in gaps {
        // 이 틈에 들어 있는 말소리의 범위와 분량
        let inside: Vec<(u64, u64)> = speech
            .iter()
            .map(|&(s, e)| (s.max(gap_start), e.min(gap_end)))
            .filter(|(s, e)| e > s)
            .collect();
        let amount: u64 = inside.iter().map(|(s, e)| e - s).sum();
        if amount >= UNRECOGNIZED_MIN_MS {
            out.push((inside[0].0, inside[inside.len() - 1].1));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Segment;

    fn seg(start: u64, end: u64) -> Segment {
        Segment { start_ms: start, end_ms: end, text: "말".into() }
    }

    #[test]
    fn span_scales_progress() {
        let span = Span { start: 0.25, end: 0.75 };
        assert_eq!((span.overall(0), span.overall(50), span.overall(100)), (25, 50, 75));
        assert_eq!(span.overall(250), 75);
    }

    #[test]
    fn reports_speech_without_text() {
        // 말소리: 0~10분, 20~80분. 글은 0~10분과 59분까지만 있고 그 뒤 21분이 비었다.
        let min = 60_000;
        let speech = [(0, 10 * min), (20 * min, 80 * min)];
        let segments = [seg(1000, 10 * min), seg(20 * min, 59 * min)];
        assert_eq!(unrecognized_speech(&speech, &segments), vec![(59 * min, 80 * min)]);
    }

    #[test]
    fn ignores_short_gaps_and_silence() {
        let min = 60_000;
        // 글 사이가 30분 비었지만 그 사이에는 말소리가 없다 → 알리지 않는다.
        let speech = [(0, 5 * min), (35 * min, 40 * min)];
        let segments = [seg(0, 5 * min), seg(35 * min, 40 * min - 20_000)];
        assert!(unrecognized_speech(&speech, &segments).is_empty());
        // 말소리가 전혀 받아 적히지 않은 경우
        assert_eq!(unrecognized_speech(&speech, &[]), vec![(0, 40 * min)]);
        assert!(unrecognized_speech(&[], &[]).is_empty());
    }
}

/// 말소리 구간을 인식 묶음으로 나눈 것. 말소리 감지를 쓸 수 없으면 None (음성 전체를 그대로 인식).
/// 말소리가 전혀 없으면 빈 목록: 엔진에 넘기면 없는 말을 지어낼 뿐이므로 아무것도 넘기지 않는다.
fn speech_blocks(
    samples: &[f32],
    progress: &(dyn Fn(i32) + Sync),
    cancel: &crate::Cancel,
) -> Option<Vec<Vec<(usize, usize)>>> {
    #[cfg(feature = "local-stt")]
    {
        let probs = vad::speech_probabilities(samples, progress, cancel).ok()?;
        Some(vad::blocks(&vad::regions_from_probs(&probs, samples.len()), &probs))
    }
    #[cfg(not(feature = "local-stt"))]
    {
        let _ = (samples, progress, cancel);
        None
    }
}
