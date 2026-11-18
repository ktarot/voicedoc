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
    let mut warnings = Vec::new();
    // 말소리 구간. 건너뛰기를 꺼도 구해 둔다: 빠진 곳을 찾아 다시 인식하고 경고하는 데 쓴다.
    let speech = match speech_regions(samples, scan_progress, cancel) {
        Ok(found) => found,
        Err(crate::Error::Cancelled) => return Err(crate::Error::Cancelled),
        Err(e) => {
            warnings.push(format!(
                "말소리 감지를 쓰지 못해 음성 전체를 그대로 인식했습니다. 빠진 문장이 있을 수 있습니다 ({e})"
            ));
            None
        }
    };
    cancel.check()?;

    let blocks: Vec<Vec<(usize, usize)>> = match (&speech, opts.keep_silence) {
        (Some((regions, probs)), false) => vad::blocks(regions, probs),
        // 건너뛰기를 끄거나 말소리 감지를 못 쓰면 음성 전체를 10분씩 그대로 넘긴다.
        _ => cloud::split_points(samples).into_iter().map(|r| vec![r]).collect(),
    };
    let speech_list = speech.as_ref().map(|(r, _)| r.as_slice());

    let total: usize = blocks.iter().flatten().map(|(s, e)| e - s).sum();
    let mut language = opts.language.clone();
    let mut segments: Vec<crate::Segment> = Vec::new();
    let mut done = 0usize;
    for regions in &blocks {
        cancel.check()?;
        let len: usize = regions.iter().map(|(s, e)| e - s).sum();
        let span = Span { start: done as f32 / total as f32, end: (done + len) as f32 / total as f32 };
        done += len;
        let Some(part) = run_engine(samples, regions, span, &mut engine)? else { continue };
        if language.is_none() {
            language = part.language;
        }
        let mut block_segments = part.segments;

        // 엔진이 말소리 일부를 통째로 건너뛰는 일이 있다 (특히 large-v3-turbo). 빈 곳만 잘라 한 번 더 인식한다.
        let end_span = Span { start: span.end, end: span.end };
        for retry in skipped_regions(regions, speech_list, &block_segments) {
            cancel.check()?;
            if let Some(again) = run_engine(samples, &retry, end_span, &mut engine)? {
                block_segments.extend(again.segments);
            }
        }
        block_segments.sort_by_key(|s| s.start_ms);
        segments.extend(block_segments);
    }
    // 문제를 살필 때: VOICEDOC_DEBUG_SEGMENTS를 주면 정리하기 전의 인식 결과를 표준 오류로 보여 준다.
    if std::env::var_os("VOICEDOC_DEBUG_SEGMENTS").is_some() {
        for seg in &segments {
            eprintln!("[원본 {:>7}~{:>7}ms] {}", seg.start_ms, seg.end_ms, seg.text);
        }
    }
    let segments = clean::clean(segments, opts.prompt.as_deref());
    let ms = |x: usize| crate::audio::duration_ms(x);
    let speech_ms: Vec<(u64, u64)> =
        speech_list.unwrap_or_default().iter().map(|&(s, e)| (ms(s), ms(e))).collect();
    Ok(crate::Transcript {
        language,
        duration_ms: crate::audio::duration_ms(samples.len()),
        unrecognized: unrecognized_speech(&speech_ms, &segments),
        warnings,
        segments,
    })
}

/// 말소리가 이만큼 이어졌는데 글이 하나도 없으면 받아 적지 못한 것으로 본다 (20초).
const UNRECOGNIZED_MIN_MS: u64 = 20_000;

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

    const SEC: usize = 16_000;

    #[test]
    fn finds_skipped_speech() {
        // 엔진에 0~31초를 넘겼는데 0~4.3초와 30~31초만 받아 적었다 (실제로 겪은 경우).
        let block = [(0, 31 * SEC)];
        let speech = [(0, 31 * SEC)];
        let segments = [seg(0, 4_300), seg(30_000, 31_000)];
        let r = skipped_regions(&block, Some(&speech), &segments);
        assert_eq!(r.len(), 1, "{r:?}");
        assert_eq!(r[0], vec![(4_300 * SEC / 1000, 30 * SEC)]);
        // 말소리 감지 없이도 8초 넘게 비면 다시 인식한다
        assert_eq!(skipped_regions(&block, None, &segments).len(), 1);
    }

    #[test]
    fn does_not_retry_silence_or_short_gaps() {
        let block = [(0, 60 * SEC)];
        // 10~50초가 비었지만 그 사이에 말소리가 없다
        let speech = [(0, 10 * SEC), (50 * SEC, 60 * SEC)];
        let segments = [seg(0, 10_000), seg(50_000, 60_000)];
        assert!(skipped_regions(&block, Some(&speech), &segments).is_empty());
        // 말소리가 2초뿐이면 다시 인식하지 않는다
        let speech = [(0, 10 * SEC), (30 * SEC, 32 * SEC), (50 * SEC, 60 * SEC)];
        assert!(skipped_regions(&block, Some(&speech), &segments).is_empty());
        // 글 사이가 3초면 건너뛴 것으로 보지 않는다
        let segments = [seg(0, 27_000), seg(30_000, 60_000)];
        assert!(skipped_regions(&block, Some(&[(0, 60 * SEC)]), &segments).is_empty());
        // 말소리 감지 없이 5초 빈 것은 조용한 곳일 수 있어 다시 인식하지 않는다
        let segments = [seg(0, 20_000), seg(25_000, 60_000)];
        assert!(skipped_regions(&block, None, &segments).is_empty());
    }

    #[test]
    fn retries_only_voiced_parts_of_the_block() {
        // 묶음은 말소리 두 조각(0~10초, 40~70초), 엔진은 0~10초만 받아 적었다.
        let block = [(0, 10 * SEC), (40 * SEC, 70 * SEC)];
        let speech = [(0, 10 * SEC), (40 * SEC, 70 * SEC)];
        let r = skipped_regions(&block, Some(&speech), &[seg(0, 10_000)]);
        assert_eq!(r, vec![vec![(40 * SEC, 70 * SEC)]]);
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
        let segments = [seg(0, 5 * min), seg(35 * min, 40 * min - 10_000)];
        assert!(unrecognized_speech(&speech, &segments).is_empty());
        // 말소리가 전혀 받아 적히지 않은 경우
        assert_eq!(unrecognized_speech(&speech, &[]), vec![(0, 40 * min)]);
        assert!(unrecognized_speech(&[], &[]).is_empty());
    }
}

/// 원래 음성의 `regions`만 이어 붙여 엔진에 넘기고, 결과 시각을 원래 위치로 되돌린다.
fn run_engine(
    samples: &[f32],
    regions: &[(usize, usize)],
    span: Span,
    engine: &mut impl FnMut(&[f32], Span) -> crate::Result<crate::Transcript>,
) -> crate::Result<Option<crate::Transcript>> {
    let mut packed = vad::Packed::new(samples, regions);
    if packed.is_empty() {
        return Ok(None);
    }
    if packed.samples.len() < MIN_ENGINE_SAMPLES {
        packed.samples.resize(MIN_ENGINE_SAMPLES, 0.0);
    }
    let mut part = engine(&packed.samples, span)?;
    for seg in &mut part.segments {
        let (start, end) = (seg.start_ms, seg.end_ms);
        seg.start_ms = packed.start_to_original_ms(start);
        seg.end_ms = packed.end_to_original_ms(end).max(seg.start_ms);
    }
    Ok(Some(part))
}

const RATE: usize = crate::audio::SAMPLE_RATE as usize;
/// 글 사이가 이보다 짧으면 건너뛴 것으로 보지 않는다.
const SKIP_MIN_GAP: usize = RATE * 4;
/// 빈 곳에 말소리가 이만큼은 있어야 다시 인식한다 (말소리 감지를 쓸 수 있을 때).
const SKIP_MIN_SPEECH: usize = RATE * 3;
/// 말소리 감지를 못 쓸 때는 빈 곳이 이만큼 길어야 다시 인식한다.
/// 조용한 곳을 엔진에 다시 넘기면 없는 말을 지어낼 수 있어 기준을 높게 둔다.
const SKIP_MIN_GAP_UNKNOWN: usize = RATE * 8;

/// 엔진이 받아 적지 않고 건너뛴 말소리 부분을 찾는다 (원래 음성의 표본 위치).
///
/// `block`은 엔진에 넘긴 구간, `speech`는 말소리 감지 결과(없으면 None), `segments`는 엔진 결과(원래 시각)다.
/// 돌려주는 목록 하나하나가 다시 인식할 묶음이다.
pub fn skipped_regions(
    block: &[(usize, usize)],
    speech: Option<&[(usize, usize)]>,
    segments: &[crate::Segment],
) -> Vec<Vec<(usize, usize)>> {
    let (Some(first), Some(last)) = (block.first(), block.last()) else { return Vec::new() };
    let pos = |ms: u64| (ms as usize).saturating_mul(RATE) / 1000;

    // 글이 없는 틈: 묶음 시작~첫 글, 글 사이, 마지막 글~묶음 끝
    let mut gaps = Vec::new();
    let mut cursor = first.0;
    let mut sorted: Vec<&crate::Segment> = segments.iter().collect();
    sorted.sort_by_key(|s| s.start_ms);
    for seg in sorted {
        let start = pos(seg.start_ms);
        if start > cursor {
            gaps.push((cursor, start));
        }
        cursor = cursor.max(pos(seg.end_ms));
    }
    if last.1 > cursor {
        gaps.push((cursor, last.1));
    }

    let clip = |list: &[(usize, usize)], g: (usize, usize)| -> Vec<(usize, usize)> {
        list.iter().map(|&(s, e)| (s.max(g.0), e.min(g.1))).filter(|(s, e)| e > s).collect()
    };
    gaps.into_iter()
        .filter(|(s, e)| e - s >= SKIP_MIN_GAP)
        .filter_map(|gap| {
            // 엔진에 넘긴 부분 가운데 이 틈에 든 것
            let inside = clip(block, gap);
            let (pieces, needed) = match speech {
                Some(speech) => {
                    let voiced: Vec<_> = clip(speech, gap).into_iter().flat_map(|r| clip(&inside, r)).collect();
                    (voiced, SKIP_MIN_SPEECH)
                }
                None => (inside, SKIP_MIN_GAP_UNKNOWN),
            };
            let amount: usize = pieces.iter().map(|(s, e)| e - s).sum();
            (amount >= needed).then_some(pieces)
        })
        .collect()
}

/// 말소리 구간과 프레임별 말소리 확률. 이 빌드에 말소리 감지가 없으면 None.
fn speech_regions(
    samples: &[f32],
    progress: &(dyn Fn(i32) + Sync),
    cancel: &crate::Cancel,
) -> crate::Result<Option<(Vec<(usize, usize)>, Vec<f32>)>> {
    #[cfg(feature = "local-stt")]
    {
        // 말소리 감지도 인식 엔진(ggml)으로 CPU에서 돈다.
        check_cpu()?;
        let probs = vad::speech_probabilities(samples, progress, cancel)?;
        Ok(Some((vad::regions_from_probs(&probs, samples.len()), probs)))
    }
    #[cfg(not(feature = "local-stt"))]
    {
        let _ = (samples, progress, cancel);
        Ok(None)
    }
}
