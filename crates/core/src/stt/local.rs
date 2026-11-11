//! whisper.cpp를 이용한 오프라인 음성 인식.

use std::ffi::c_void;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Once;

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use super::SttOptions;
use crate::audio;
use crate::{Cancel, Error, Result, Segment, Transcript};

static LOG_HOOK: Once = Once::new();

fn default_threads() -> i32 {
    let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    n.clamp(1, 8) as i32
}

unsafe extern "C" fn abort_trampoline(user_data: *mut c_void) -> bool {
    (*(user_data as *const AtomicBool)).load(Ordering::SeqCst)
}

/// 16kHz 모노 샘플을 텍스트로 바꾼다. `progress`에는 0~100 값이 전달된다.
pub fn transcribe(
    model_path: &Path,
    samples: &[f32],
    opts: &SttOptions,
    progress: impl FnMut(i32) + 'static,
    cancel: &Cancel,
) -> Result<Transcript> {
    // whisper.cpp가 표준 오류로 쏟아내는 기록을 막는다.
    LOG_HOOK.call_once(whisper_rs::install_logging_hooks);

    if !model_path.is_file() {
        return Err(Error::Model("인식 모델 파일이 없습니다. 먼저 모델을 내려받아 주세요".into()));
    }
    // whisper.cpp는 윈도에서 한글이 들어간 경로(예: C:\\Users\\홍길동)의 파일을 열지 못한다.
    // 그래서 파일을 직접 열어 메모리 맵으로 넘긴다. 맵은 읽기 전용이며 불러오는 동안만 쓴다.
    let file = std::fs::File::open(model_path)?;
    // 안전성: 모델 파일은 앱 전용 폴더에 있고 불러오는 동안 수정하지 않는다.
    let map = unsafe { memmap2::Mmap::map(&file) }?;
    let ctx = WhisperContext::new_from_buffer_with_params(&map, WhisperContextParameters::default())
        .map_err(|e| Error::Model(format!("모델을 불러오지 못했습니다 (파일이 손상되었을 수 있습니다): {e}")))?;
    drop(map);
    let mut state = ctx.create_state().map_err(|e| Error::Stt(e.to_string()))?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_n_threads(default_threads());
    params.set_language(Some(opts.language.as_deref().unwrap_or("auto")));
    params.set_translate(false);
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_suppress_blank(true);
    if let Some(prompt) = opts.prompt.as_deref().filter(|p| !p.trim().is_empty()) {
        params.set_initial_prompt(prompt);
    }
    params.set_progress_callback_safe(progress);
    // whisper-rs 0.14의 set_abort_callback_safe는 클로저를 잘못된 타입으로 호출하는
    // 버그가 있어 쓰지 않는다. 취소 깃발의 주소를 직접 넘긴다.
    // 안전성: `cancel`은 아래 `state.full` 호출이 끝날 때까지 살아 있다.
    unsafe {
        params.set_abort_callback(Some(abort_trampoline));
        params.set_abort_callback_user_data(cancel.flag() as *const AtomicBool as *mut c_void);
    }

    let run = state.full(params, samples);
    cancel.check()?;
    run.map_err(|e| Error::Stt(e.to_string()))?;

    let n = state.full_n_segments().map_err(|e| Error::Stt(e.to_string()))?;
    let mut segments = Vec::with_capacity(n as usize);
    for i in 0..n {
        let text = state
            .full_get_segment_text_lossy(i)
            .map_err(|e| Error::Stt(e.to_string()))?;
        // whisper 시각 단위는 10ms
        let t0 = state.full_get_segment_t0(i).map_err(|e| Error::Stt(e.to_string()))?;
        let t1 = state.full_get_segment_t1(i).map_err(|e| Error::Stt(e.to_string()))?;
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        segments.push(Segment {
            start_ms: t0.max(0) as u64 * 10,
            end_ms: t1.max(0) as u64 * 10,
            text: text.to_string(),
        });
    }
    let language = match opts.language.clone() {
        Some(l) => Some(l),
        None => state
            .full_lang_id_from_state()
            .ok()
            .and_then(whisper_rs::get_lang_str)
            .map(str::to_string),
    };
    Ok(Transcript { language, duration_ms: audio::duration_ms(samples.len()), segments })
}
