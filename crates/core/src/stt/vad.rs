//! 말소리 구간 찾기(VAD)와, 말소리만 이어 붙인 음성 만들기.
//!
//! Whisper는 침묵·잡음·음악 구간에서 없는 말을 지어내거나 같은 문장을 되풀이한다.
//! 그래서 인식 전에 말소리 구간만 골라 이어 붙이고, 인식 결과의 시각은 원래 위치로 되돌린다.

use crate::audio::SAMPLE_RATE;

/// 확률 하나가 대표하는 표본 수 (Silero VAD의 처리 단위, 32ms).
pub const FRAME: usize = 512;

const RATE: usize = SAMPLE_RATE as usize;
/// 이 확률 이상이면 말소리가 시작된 것으로 본다.
const START_THRESHOLD: f32 = 0.5;
/// 말소리 도중에는 이 확률 아래로 내려가야 끊긴 것으로 본다.
const END_THRESHOLD: f32 = 0.35;
/// 이보다 짧게 끊긴 것은 숨 고르기로 보고 잇는다.
const MIN_SILENCE: usize = RATE * 6 / 10;
/// 이보다 짧은 소리는 말소리로 치지 않는다.
const MIN_SPEECH: usize = RATE / 4;
/// 첫소리·끝소리가 잘리지 않도록 앞뒤에 붙이는 여유.
const PAD: usize = RATE / 4;
/// 구간 사이가 이보다 짧으면 사이의 원래 소리를 그대로 둔다.
const MERGE_GAP: usize = RATE;
/// 이어 붙일 때 구간 사이에 넣는 침묵.
const JOIN_SILENCE: usize = RATE / 2;

/// 프레임별 말소리 확률에서 말소리 구간(표본 위치의 시작·끝)을 구한다.
pub fn regions_from_probs(probs: &[f32], total_samples: usize) -> Vec<(usize, usize)> {
    let mut raw: Vec<(usize, usize)> = Vec::new();
    let mut start: Option<usize> = None;
    let mut last_voiced = 0usize;
    for (i, &p) in probs.iter().enumerate() {
        let pos = i * FRAME;
        match start {
            None if p >= START_THRESHOLD => {
                start = Some(pos);
                last_voiced = pos + FRAME;
            }
            Some(s) => {
                if p >= END_THRESHOLD {
                    last_voiced = pos + FRAME;
                } else if pos + FRAME - last_voiced >= MIN_SILENCE {
                    raw.push((s, last_voiced));
                    start = None;
                }
            }
            None => {}
        }
    }
    if let Some(s) = start {
        raw.push((s, last_voiced));
    }

    let mut out: Vec<(usize, usize)> = Vec::new();
    for (s, e) in raw {
        if e - s < MIN_SPEECH {
            continue;
        }
        let s = s.saturating_sub(PAD);
        let e = (e + PAD).min(total_samples);
        match out.last_mut() {
            Some(prev) if s <= prev.1 + MERGE_GAP => prev.1 = prev.1.max(e),
            _ => out.push((s, e)),
        }
    }
    out
}

#[derive(Debug, Clone, Copy)]
struct Piece {
    /// 이어 붙인 음성에서의 시작 위치
    packed_start: usize,
    /// 원래 음성에서의 시작 위치
    orig_start: usize,
    len: usize,
}

/// 말소리 구간만 이어 붙인 음성과, 그 시각을 원래 시각으로 되돌리는 표.
#[derive(Debug, Clone, Default)]
pub struct Packed {
    pub samples: Vec<f32>,
    pieces: Vec<Piece>,
}

impl Packed {
    pub fn new(samples: &[f32], regions: &[(usize, usize)]) -> Self {
        let total: usize = regions.iter().map(|(s, e)| e - s + JOIN_SILENCE).sum();
        let mut out = Vec::with_capacity(total);
        let mut pieces = Vec::with_capacity(regions.len());
        for &(s, e) in regions {
            let e = e.min(samples.len());
            if s >= e {
                continue;
            }
            if !out.is_empty() {
                out.resize(out.len() + JOIN_SILENCE, 0.0);
            }
            pieces.push(Piece { packed_start: out.len(), orig_start: s, len: e - s });
            out.extend_from_slice(&samples[s..e]);
        }
        Self { samples: out, pieces }
    }

    pub fn is_empty(&self) -> bool {
        self.pieces.is_empty()
    }

    /// 구간이 시작하는 시각(ms)을 원래 음성의 시각으로 바꾼다.
    ///
    /// 인식 엔진의 시각은 0.2~0.3초쯤 어긋날 수 있다. 조각의 끝자락이나 조각 사이의 침묵에서
    /// 시작하는 구간은 실제로는 다음 조각의 말이므로 다음 조각의 시작으로 본다.
    pub fn start_to_original_ms(&self, packed_ms: u64) -> u64 {
        let pos = position(packed_ms);
        let idx = self.pieces.partition_point(|p| p.packed_start <= pos);
        let Some(i) = idx.checked_sub(1) else {
            return self.pieces.first().map(|p| ms(p.orig_start)).unwrap_or(0);
        };
        let piece = self.pieces[i];
        match self.pieces.get(i + 1) {
            Some(next) if pos + SNAP >= piece.packed_start + piece.len => ms(next.orig_start),
            _ => ms(piece.orig_start + (pos - piece.packed_start).min(piece.len)),
        }
    }

    /// 구간이 끝나는 시각(ms)을 원래 음성의 시각으로 바꾼다.
    /// 조각 사이의 침묵이나 다음 조각의 첫머리에서 끝나면 앞 조각의 끝으로 본다.
    pub fn end_to_original_ms(&self, packed_ms: u64) -> u64 {
        let pos = position(packed_ms);
        let idx = self.pieces.partition_point(|p| p.packed_start + SNAP < pos);
        let Some(piece) = idx.checked_sub(1).and_then(|i| self.pieces.get(i)) else {
            return self.pieces.first().map(|p| ms(p.orig_start)).unwrap_or(0);
        };
        ms(piece.orig_start + (pos - piece.packed_start).min(piece.len))
    }
}

/// 인식 엔진 시각의 오차로 보는 범위 (0.3초).
const SNAP: usize = RATE * 3 / 10;

fn position(packed_ms: u64) -> usize {
    (packed_ms as usize).saturating_mul(RATE) / 1000
}

fn ms(samples: usize) -> u64 {
    samples as u64 * 1000 / RATE as u64
}

/// 한 번에 처리하는 길이 (5분). 이 단위로 여러 코어에 나눠 맡기고 진행률·취소를 확인한다.
#[cfg(feature = "local-stt")]
const CHUNK_FRAMES: usize = 9375;
/// 조각 앞에 덧붙여 읽는 길이 (약 1초). 감지 모델은 앞 소리를 기억하며 판단하므로,
/// 조각 첫머리의 판단이 흔들리지 않도록 미리 들려준 뒤 그 부분의 결과는 버린다.
#[cfg(feature = "local-stt")]
const WARMUP_FRAMES: usize = 32;

/// 프레임별 말소리 확률을 구한다 (Silero VAD, CPU의 여러 코어로 나눠 실행).
/// `progress`에는 0~100 값이 전달된다.
#[cfg(feature = "local-stt")]
pub fn speech_probabilities(
    samples: &[f32],
    progress: &(dyn Fn(i32) + Sync),
    cancel: &crate::Cancel,
) -> crate::Result<Vec<f32>> {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;
    use whisper_rs::whisper_rs_sys as sys;

    use crate::Error;

    /// 실행 파일에 넣어 둔 모델(약 0.9MB, MIT). 파일 경로를 거치지 않으므로
    /// 한글이 들어간 사용자 폴더에서도 문제없이 읽힌다.
    static MODEL: &[u8] = include_bytes!("../../assets/ggml-silero-v5.1.2.bin");

    struct Reader {
        pos: usize,
    }
    unsafe extern "C" fn read(ctx: *mut c_void, output: *mut c_void, size: usize) -> usize {
        let r = &mut *(ctx as *mut Reader);
        let n = size.min(MODEL.len() - r.pos);
        std::ptr::copy_nonoverlapping(MODEL.as_ptr().add(r.pos), output as *mut u8, n);
        r.pos += n;
        n
    }
    unsafe extern "C" fn eof(ctx: *mut c_void) -> bool {
        (*(ctx as *const Reader)).pos >= MODEL.len()
    }
    unsafe extern "C" fn close(_ctx: *mut c_void) {}

    /// 감지 모델 하나. 스레드마다 따로 만든다.
    struct Detector(*mut sys::whisper_vad_context);
    impl Detector {
        fn new() -> Option<Self> {
            let mut reader = Reader { pos: 0 };
            // 안전성: `reader`는 모델을 다 읽는 이 호출 동안만 쓰인다.
            let ctx = unsafe {
                let mut loader = sys::whisper_model_loader {
                    context: &mut reader as *mut Reader as *mut c_void,
                    read: Some(read),
                    eof: Some(eof),
                    close: Some(close),
                };
                let mut params = sys::whisper_vad_default_context_params();
                // 계산이 아주 작아서 스레드를 나누면 오히려 느리다. 대신 조각을 여러 코어에 나눈다.
                params.n_threads = 1;
                params.use_gpu = false;
                sys::whisper_vad_init_with_params(&mut loader, params)
            };
            (!ctx.is_null()).then_some(Self(ctx))
        }

        fn run(&mut self, samples: &[f32]) -> Option<Vec<f32>> {
            // 안전성: 확률 배열은 다음 호출이나 해제 전에 복사한다.
            unsafe {
                let ok = sys::whisper_vad_detect_speech(self.0, samples.as_ptr(), samples.len() as i32);
                let n = sys::whisper_vad_n_probs(self.0);
                let ptr = sys::whisper_vad_probs(self.0);
                (ok && n > 0 && !ptr.is_null()).then(|| std::slice::from_raw_parts(ptr, n as usize).to_vec())
            }
        }
    }
    impl Drop for Detector {
        fn drop(&mut self) {
            unsafe { sys::whisper_vad_free(self.0) }
        }
    }

    super::quiet_engine_logs();
    let _running = super::engine_lock();
    if samples.is_empty() {
        return Ok(Vec::new());
    }

    let chunk = CHUNK_FRAMES * FRAME;
    let n_chunks = samples.len().div_ceil(chunk);
    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 8).min(n_chunks);
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let failed = AtomicBool::new(false);
    let results: Mutex<Vec<Option<Vec<f32>>>> = Mutex::new(vec![None; n_chunks]);

    progress(0);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                let Some(mut detector) = Detector::new() else {
                    failed.store(true, Ordering::SeqCst);
                    return;
                };
                loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    if i >= n_chunks || cancel.is_cancelled() || failed.load(Ordering::SeqCst) {
                        return;
                    }
                    let start = i * chunk;
                    let end = (start + chunk).min(samples.len());
                    let warmup = if i == 0 { 0 } else { WARMUP_FRAMES };
                    match detector.run(&samples[start - warmup * FRAME..end]) {
                        Some(mut probs) => {
                            probs.drain(..warmup.min(probs.len()));
                            results.lock().unwrap_or_else(|e| e.into_inner())[i] = Some(probs);
                            let n = done.fetch_add(1, Ordering::SeqCst) + 1;
                            progress((n * 100 / n_chunks) as i32);
                        }
                        None => {
                            failed.store(true, Ordering::SeqCst);
                            return;
                        }
                    }
                }
            });
        }
    });
    cancel.check()?;
    let results = results.into_inner().unwrap_or_else(|e| e.into_inner());
    if failed.load(Ordering::SeqCst) || results.iter().any(Option::is_none) {
        return Err(Error::Stt("말소리 구간을 찾지 못했습니다".into()));
    }
    Ok(results.into_iter().flatten().flatten().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probs(spec: &[(f32, f32)]) -> Vec<f32> {
        // (초, 확률) 목록을 프레임 배열로
        spec.iter()
            .flat_map(|&(secs, p)| std::iter::repeat(p).take((secs * RATE as f32) as usize / FRAME))
            .collect()
    }

    #[test]
    fn finds_speech_between_silences() {
        let p = probs(&[(10.0, 0.01), (5.0, 0.9), (20.0, 0.02), (3.0, 0.8), (8.0, 0.0)]);
        let total = p.len() * FRAME;
        let r = regions_from_probs(&p, total);
        assert_eq!(r.len(), 2);
        let secs = |x: usize| x as f32 / RATE as f32;
        assert!((secs(r[0].0) - 9.75).abs() < 0.1 && (secs(r[0].1) - 15.25).abs() < 0.1, "{r:?}");
        assert!((secs(r[1].0) - 34.75).abs() < 0.1 && (secs(r[1].1) - 38.25).abs() < 0.1, "{r:?}");
    }

    #[test]
    fn keeps_short_pauses_and_drops_clicks() {
        // 0.3초 숨 고르기는 잇고, 0.1초짜리 소리는 버린다.
        let p = probs(&[(2.0, 0.9), (0.3, 0.1), (2.0, 0.9), (5.0, 0.0), (0.1, 0.9), (5.0, 0.0)]);
        let r = regions_from_probs(&p, p.len() * FRAME);
        assert_eq!(r.len(), 1, "{r:?}");
        assert!(regions_from_probs(&probs(&[(30.0, 0.1)]), 30 * RATE).is_empty());
        assert!(regions_from_probs(&[], 0).is_empty());
    }

    #[test]
    fn maps_time_back_to_original() {
        let samples: Vec<f32> = (0..RATE * 60).map(|i| i as f32).collect();
        let regions = [(RATE * 10, RATE * 15), (RATE * 40, RATE * 43)];
        let packed = Packed::new(&samples, &regions);
        // 5초 + 침묵 0.5초 + 3초
        assert_eq!(packed.samples.len(), RATE * 8 + RATE / 2);
        assert_eq!(packed.samples[0], (RATE * 10) as f32);
        assert_eq!(packed.start_to_original_ms(0), 10_000);
        assert_eq!(packed.start_to_original_ms(2_500), 12_500);
        assert_eq!(packed.end_to_original_ms(2_500), 12_500);
        assert_eq!(packed.end_to_original_ms(5_000), 15_000);
        // 조각 사이 침묵: 시작은 다음 조각으로, 끝은 앞 조각으로
        assert_eq!(packed.start_to_original_ms(5_300), 40_000);
        assert_eq!(packed.end_to_original_ms(5_300), 15_000);
        // 엔진 시각 오차(0.3초 이내)도 같은 쪽으로 붙인다
        assert_eq!(packed.start_to_original_ms(4_800), 40_000);
        assert_eq!(packed.end_to_original_ms(5_700), 15_000);
        assert_eq!(packed.start_to_original_ms(5_500), 40_000);
        assert_eq!(packed.end_to_original_ms(7_000), 41_500);
        // 마지막 조각의 끝자락에서 시작해도 뒤로 넘어가지 않고, 끝을 넘어가도 벗어나지 않는다
        assert_eq!(packed.start_to_original_ms(8_400), 42_900);
        assert_eq!(packed.end_to_original_ms(99_000), 43_000);
        assert!(Packed::new(&samples, &[]).is_empty());
    }
}
