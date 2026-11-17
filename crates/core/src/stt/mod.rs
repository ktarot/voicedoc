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

/// whisper.cpp가 표준 오류로 쏟아내는 기록을 막는다.
#[cfg(feature = "local-stt")]
pub(crate) fn quiet_engine_logs() {
    static HOOK: std::sync::Once = std::sync::Once::new();
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

/// 말소리 구간만 골라 `engine`으로 인식하고, 시각을 원래 위치로 되돌린 뒤 결과를 정리한다.
///
/// 로컬·클라우드 인식 모두 이 함수를 거친다. `engine`은 16kHz 모노 표본을 받아 인식한다.
pub fn transcribe_speech(
    samples: &[f32],
    opts: &SttOptions,
    cancel: &crate::Cancel,
    engine: impl FnOnce(&[f32]) -> crate::Result<crate::Transcript>,
) -> crate::Result<crate::Transcript> {
    let duration_ms = crate::audio::duration_ms(samples.len());
    let packed = if opts.keep_silence { None } else { pack_speech(samples) };
    cancel.check()?;

    let mut transcript = match &packed {
        Some(packed) if packed.is_empty() => {
            // 말소리가 전혀 없다: 엔진에 넘기면 없는 말을 지어낼 뿐이다.
            return Ok(crate::Transcript { language: opts.language.clone(), duration_ms, segments: Vec::new() });
        }
        Some(packed) if packed.samples.len() < MIN_ENGINE_SAMPLES => {
            let mut padded = packed.samples.clone();
            padded.resize(MIN_ENGINE_SAMPLES, 0.0);
            engine(&padded)?
        }
        Some(packed) => engine(&packed.samples)?,
        None => engine(samples)?,
    };
    if let Some(packed) = &packed {
        for seg in &mut transcript.segments {
            let (start, end) = (seg.start_ms, seg.end_ms);
            seg.start_ms = packed.start_to_original_ms(start);
            seg.end_ms = packed.end_to_original_ms(end).max(seg.start_ms);
        }
    }
    transcript.duration_ms = duration_ms;
    transcript.segments = clean::clean(transcript.segments, opts.prompt.as_deref());
    Ok(transcript)
}

/// 말소리만 이어 붙인 음성. 말소리 감지를 쓸 수 없으면 None (음성 전체를 그대로 인식).
fn pack_speech(samples: &[f32]) -> Option<vad::Packed> {
    #[cfg(feature = "local-stt")]
    {
        let probs = vad::speech_probabilities(samples).ok()?;
        Some(vad::Packed::new(samples, &vad::regions_from_probs(&probs, samples.len())))
    }
    #[cfg(not(feature = "local-stt"))]
    {
        let _ = samples;
        None
    }
}
