//! whisper.cpp를 이용한 오프라인 음성 인식.

use std::ffi::c_void;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState};

use super::SttOptions;
use crate::audio;
use crate::{Cancel, Error, Result, Segment, Transcript};

fn default_threads() -> i32 {
    let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    n.clamp(1, 8) as i32
}

/// 그래픽카드 가속을 쓸 수 있으면 그 장치 이름을 돌려준다. 없거나 CPU 전용 빌드면 None.
pub fn accelerator() -> Option<String> {
    #[cfg(feature = "vulkan")]
    {
        use whisper_rs::whisper_rs_sys as sys;
        prepare_vulkan();
        // ggml 장치 목록은 Vulkan 오류를 내부에서 처리한다. whisper.cpp도 이 목록의 첫 GPU를 쓴다.
        // 안전성: 장치 포인터와 설명 문자열은 ggml이 프로그램이 끝날 때까지 보관한다.
        unsafe {
            (0..sys::ggml_backend_dev_count())
                .map(|i| sys::ggml_backend_dev_get(i))
                .find(|&d| {
                    !d.is_null()
                        && sys::ggml_backend_dev_type(d) == sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_GPU
                })
                .map(|d| std::ffi::CStr::from_ptr(sys::ggml_backend_dev_description(d)).to_string_lossy().into_owned())
        }
    }
    #[cfg(not(feature = "vulkan"))]
    {
        None
    }
}

/// 윈도에서 vulkan-1.dll(그래픽 드라이버가 설치)이 없으면 인식 엔진이 Vulkan을 건너뛰게 한다.
/// 앱은 이 DLL을 지연 로드하므로, 확인 없이 쓰면 드라이버 없는 PC에서 종료된다.
#[cfg(feature = "vulkan")]
fn prepare_vulkan() {
    static CHECK: std::sync::Once = std::sync::Once::new();
    CHECK.call_once(|| {
        #[cfg(windows)]
        {
            #[link(name = "kernel32")]
            extern "system" {
                fn LoadLibraryW(name: *const u16) -> *mut c_void;
            }
            let name: Vec<u16> = "vulkan-1.dll".encode_utf16().chain(Some(0)).collect();
            // 안전성: 0으로 끝나는 UTF-16 문자열을 넘긴다. 불러온 DLL은 프로그램이 끝날 때까지 둔다.
            if unsafe { LoadLibraryW(name.as_ptr()) }.is_null() {
                std::env::set_var("VOICEDOC_NO_VULKAN", "1");
            }
        }
    });
}

unsafe extern "C" fn abort_trampoline(user_data: *mut c_void) -> bool {
    (*(user_data as *const AtomicBool)).load(Ordering::SeqCst)
}

/// 마지막으로 불러온 모델. 여러 파일을 연달아 인식할 때 매번 다시 읽어 그래픽카드에 올리지 않도록 둔다.
/// 다른 모델이나 다른 장치 설정으로 인식하면 바꿔 든다.
struct Loaded {
    path: std::path::PathBuf,
    /// 같은 이름으로 파일이 바뀌었는지 알아보기 위한 크기와 수정 시각
    stamp: (u64, Option<std::time::SystemTime>),
    want_gpu: bool,
    ctx: WhisperContext,
}

static LOADED: std::sync::Mutex<Option<Loaded>> = std::sync::Mutex::new(None);

/// 불러 둔 모델을 내려놓는다 (모델 파일을 지울 때 등).
pub fn unload() {
    *LOADED.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

fn new_state(model_path: &Path, want_gpu: bool) -> Result<WhisperState> {
    let meta = std::fs::metadata(model_path)?;
    let stamp = (meta.len(), meta.modified().ok());
    let mut loaded = LOADED.lock().unwrap_or_else(|e| e.into_inner());
    let reusable = loaded
        .as_ref()
        .is_some_and(|l| l.path == model_path && l.stamp == stamp && l.want_gpu == want_gpu);
    if !reusable {
        // 새 모델을 올리기 전에 먼저 내려놓아 메모리를 두 배로 쓰지 않게 한다.
        *loaded = None;
        // whisper.cpp는 윈도에서 한글이 들어간 경로(예: C:\\Users\\홍길동)의 파일을 열지 못한다.
        // 그래서 파일을 직접 열어 메모리 맵으로 넘긴다. 맵은 읽기 전용이며 불러오는 동안만 쓴다.
        let file = std::fs::File::open(model_path)?;
        // 안전성: 모델 파일은 앱 전용 폴더에 있고 불러오는 동안 수정하지 않는다.
        let map = unsafe { memmap2::Mmap::map(&file) }?;
        #[cfg(feature = "vulkan")]
        prepare_vulkan();
        let load = |use_gpu: bool| -> Result<WhisperContext> {
            let mut params = WhisperContextParameters::default();
            params.use_gpu = use_gpu;
            // whisper.cpp 1.8의 기본값. 그래픽카드에서 더 빠르고 메모리를 덜 쓴다 (whisper-rs는 기본으로 끈다).
            params.flash_attn = use_gpu;
            WhisperContext::new_from_buffer_with_params(&map, params)
                .map_err(|e| Error::Model(format!("모델을 불러오지 못했습니다 (파일이 손상되었을 수 있습니다): {e}")))
        };
        // 그래픽카드에 올리지 못하면(메모리 부족, 드라이버 문제 등) CPU로 다시 시도한다.
        let ctx = match load(want_gpu) {
            Err(_) if want_gpu => load(false)?,
            other => other?,
        };
        *loaded = Some(Loaded { path: model_path.to_path_buf(), stamp, want_gpu, ctx });
    }
    let ctx = &loaded.as_ref().expect("방금 채웠다").ctx;
    match ctx.create_state() {
        Ok(state) => Ok(state),
        Err(e) => {
            // 상태를 만들지 못하는 모델은 다음에 다시 불러오도록 버린다.
            *loaded = None;
            Err(Error::Stt(e.to_string()))
        }
    }
}

/// 16kHz 모노 샘플을 텍스트로 바꾼다. `progress`에는 0~100 값이 전달된다.
pub fn transcribe(
    model_path: &Path,
    samples: &[f32],
    opts: &SttOptions,
    progress: impl FnMut(i32) + 'static,
    cancel: &Cancel,
) -> Result<Transcript> {
    super::quiet_engine_logs();
    super::check_cpu()?;

    let _running = super::engine_lock();
    cancel.check()?;

    if !model_path.is_file() {
        return Err(Error::Model("인식 모델 파일이 없습니다. 먼저 모델을 내려받아 주세요".into()));
    }
    let want_gpu = cfg!(feature = "vulkan") && !opts.cpu_only;
    let mut state = new_state(model_path, want_gpu)?;
    cancel.check()?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_n_threads(default_threads());
    params.set_language(Some(opts.language.as_deref().unwrap_or("auto")));
    params.set_translate(false);
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_suppress_blank(true);
    // 앞에서 받아 적은 문장을 다음 구간의 참고로 넘기지 않는다. 넘기면 한 번 잘못 적힌 문장이
    // 계속 참고가 되어, 그 뒤로 끝까지 같은 말만 되풀이하는 일이 생긴다.
    params.set_no_context(true);
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

    let n = state.full_n_segments();
    let mut segments = Vec::with_capacity(n.max(0) as usize);
    for i in 0..n {
        let Some(seg) = state.get_segment(i) else { continue };
        let text = seg.to_str_lossy().map_err(|e| Error::Stt(e.to_string()))?;
        // whisper 시각 단위는 10ms
        let (t0, t1) = (seg.start_timestamp(), seg.end_timestamp());
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
        None => whisper_rs::get_lang_str(state.full_lang_id_from_state()).map(str::to_string),
    };
    Ok(Transcript { language, duration_ms: audio::duration_ms(samples.len()), segments, ..Default::default() })
}
