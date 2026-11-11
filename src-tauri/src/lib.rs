//! Tauri 명령: 화면(웹뷰)과 핵심 라이브러리를 잇는다.
//! 오래 걸리는 일은 모두 작업 스레드에서 돌리고 진행 상황은 이벤트로 알린다.

mod secrets;
mod settings;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use voicedoc_core::ai::{self, AiConfig, AiOutput, Provider, Task};
use voicedoc_core::doc::Document;
use voicedoc_core::export::{self, Format};
use voicedoc_core::stt::{cloud, SttOptions};
use voicedoc_core::{audio, model, Cancel, Error, Transcript};

use settings::{Settings, SttMode};

type CmdResult<T> = Result<T, String>;

/// 진행 중인 작업의 취소 신호. 키는 화면이 정한 작업 ID.
#[derive(Default)]
struct Jobs(Mutex<HashMap<String, Cancel>>);

impl Jobs {
    fn start(&self, id: &str) -> Cancel {
        let cancel = Cancel::new();
        self.0.lock().unwrap().insert(id.to_string(), cancel.clone());
        cancel
    }
    fn finish(&self, id: &str) {
        self.0.lock().unwrap().remove(id);
    }
}

fn config_dir(app: &AppHandle) -> CmdResult<PathBuf> {
    app.path().app_config_dir().map_err(|e| e.to_string())
}

fn models_dir(app: &AppHandle) -> CmdResult<PathBuf> {
    Ok(app.path().app_data_dir().map_err(|e| e.to_string())?.join("models"))
}

fn output_dir(app: &AppHandle, settings: &Settings) -> CmdResult<PathBuf> {
    if !settings.output_dir.trim().is_empty() {
        return Ok(PathBuf::from(settings.output_dir.trim()));
    }
    // 문서 폴더를 알 수 없는 환경(일부 리눅스, 안드로이드)에서는 홈 또는 앱 전용 폴더를 쓴다.
    let path = app.path();
    let base = path
        .document_dir()
        .or_else(|_| path.home_dir().map(|h| h.join("Documents")))
        .or_else(|_| path.app_data_dir())
        .map_err(|e| e.to_string())?;
    Ok(base.join("VoiceDoc"))
}

/// 블로킹 작업을 작업 스레드에서 실행하고 오류를 화면용 문자열로 바꾼다.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, Error> + Send + 'static,
) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("내부 오류: {e}"))?
        .map_err(|e| e.to_string())
}

// ---------- 설정 ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SettingsView {
    #[serde(flatten)]
    settings: Settings,
    has_claude_key: bool,
    has_openai_key: bool,
    /// 실제로 저장될 폴더 (설정이 비어 있을 때의 기본값 포함).
    resolved_output_dir: String,
    audio_extensions: &'static [&'static str],
}

#[tauri::command]
fn get_settings(app: AppHandle) -> CmdResult<SettingsView> {
    let dir = config_dir(&app)?;
    let settings = settings::load(&dir);
    Ok(SettingsView {
        has_claude_key: !secrets::get(&dir, Provider::Claude).is_empty(),
        has_openai_key: !secrets::get(&dir, Provider::OpenAi).is_empty(),
        resolved_output_dir: output_dir(&app, &settings)?.to_string_lossy().into_owned(),
        audio_extensions: audio::EXTENSIONS,
        settings,
    })
}

#[tauri::command]
fn save_settings(app: AppHandle, settings: Settings) -> CmdResult<()> {
    settings::save(&config_dir(&app)?, &settings)
}

/// 빈 문자열을 넘기면 저장된 키를 지운다.
#[tauri::command]
fn set_api_key(app: AppHandle, provider: Provider, key: String) -> CmdResult<()> {
    secrets::set(&config_dir(&app)?, provider, &key)
}

// ---------- 인식 모델 ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelView {
    id: &'static str,
    name: &'static str,
    size_mb: u32,
    description: &'static str,
    downloaded: bool,
}

#[tauri::command]
fn list_models(app: AppHandle) -> CmdResult<Vec<ModelView>> {
    let dir = models_dir(&app)?;
    Ok(model::MODELS
        .iter()
        .map(|m| ModelView {
            id: m.id,
            name: m.name,
            size_mb: m.size_mb,
            description: m.description,
            downloaded: model::is_downloaded(&dir, m),
        })
        .collect())
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelProgress {
    id: String,
    done: u64,
    total: Option<u64>,
}

#[tauri::command]
async fn download_model(app: AppHandle, jobs: State<'_, Jobs>, id: String) -> CmdResult<()> {
    let info = model::find(&id).ok_or("알 수 없는 모델입니다")?;
    let dir = models_dir(&app)?;
    let job_id = format!("model:{id}");
    let cancel = jobs.start(&job_id);
    let emitter = app.clone();
    let result = blocking(move || {
        let mut last_emit = 0u64;
        model::download(
            &dir,
            info,
            model::DEFAULT_BASE_URL,
            |done, total| {
                // 이벤트가 너무 잦지 않게 2MB마다 알린다.
                if done - last_emit >= 2 * 1024 * 1024 || Some(done) == total {
                    last_emit = done;
                    let _ = emitter.emit("model-progress", ModelProgress { id: info.id.into(), done, total });
                }
            },
            &cancel,
        )
        .map(|_| ())
    })
    .await;
    jobs.finish(&job_id);
    result
}

#[tauri::command]
fn delete_model(app: AppHandle, id: String) -> CmdResult<()> {
    let info = model::find(&id).ok_or("알 수 없는 모델입니다")?;
    let path = model::path_in(&models_dir(&app)?, info);
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    }
}

// ---------- 음성 인식 ----------

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct JobProgress {
    job_id: String,
    /// "decode" 또는 "transcribe"
    stage: &'static str,
    percent: i32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TranscribeResult {
    transcript: Transcript,
    text: String,
}

#[tauri::command]
async fn transcribe(
    app: AppHandle,
    jobs: State<'_, Jobs>,
    job_id: String,
    path: String,
) -> CmdResult<TranscribeResult> {
    let dir = config_dir(&app)?;
    let settings = settings::load(&dir);
    let models = models_dir(&app)?;
    let openai_key = secrets::get(&dir, Provider::OpenAi);
    let cancel = jobs.start(&job_id);
    let emitter = app.clone();
    let id = job_id.clone();

    let result = blocking(move || {
        let emit = move |stage: &'static str, percent: i32| {
            let _ = emitter.emit("job-progress", JobProgress { job_id: id.clone(), stage, percent });
        };
        let mut last = -1;
        let decode_emit = emit.clone();
        let samples = audio::decode_file(
            Path::new(&path),
            |p| {
                let pct = (p * 100.0) as i32;
                if pct != last {
                    last = pct;
                    decode_emit("decode", pct);
                }
            },
            &cancel,
        )?;
        if samples.is_empty() {
            return Err(Error::Decode("파일에 음성 데이터가 없습니다".into()));
        }

        let opts = SttOptions {
            language: Some(settings.language.trim().to_string()).filter(|l| !l.is_empty()),
            prompt: Some(settings.prompt.clone()).filter(|p| !p.trim().is_empty()),
        };
        emit("transcribe", 0);
        let transcript = match settings.stt_mode {
            SttMode::Cloud => {
                let cfg = cloud::CloudSttConfig {
                    api_key: openai_key,
                    model: settings.cloud_stt_model.clone(),
                    base_url: cloud::DEFAULT_BASE_URL.into(),
                };
                let e = emit.clone();
                cloud::transcribe(&cfg, &samples, &opts, move |p| e("transcribe", p), &cancel)?
            }
            SttMode::Local => {
                let info = model::find(&settings.model_id)
                    .ok_or_else(|| Error::Model("선택한 모델을 찾을 수 없습니다".into()))?;
                let e = emit.clone();
                voicedoc_core::stt::local::transcribe(
                    &model::path_in(&models, info),
                    &samples,
                    &opts,
                    move |p| e("transcribe", p),
                    &cancel,
                )?
            }
        };
        let text = transcript.to_text(settings.timestamps);
        Ok(TranscribeResult { transcript, text })
    })
    .await;
    jobs.finish(&job_id);
    result
}

/// 시각 표시 여부를 바꿔 본문을 다시 만든다.
#[tauri::command]
fn format_transcript(transcript: Transcript, timestamps: bool) -> String {
    transcript.to_text(timestamps)
}

#[tauri::command]
fn cancel_job(jobs: State<'_, Jobs>, job_id: String) {
    if let Some(cancel) = jobs.0.lock().unwrap().get(&job_id) {
        cancel.cancel();
    }
}

// ---------- 온라인 AI ----------

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AiDelta {
    job_id: String,
    delta: String,
}

#[tauri::command]
async fn run_ai(
    app: AppHandle,
    jobs: State<'_, Jobs>,
    job_id: String,
    task: Task,
    text: String,
) -> CmdResult<AiOutput> {
    let dir = config_dir(&app)?;
    let settings = settings::load(&dir);
    let cfg = AiConfig {
        provider: settings.provider,
        api_key: secrets::get(&dir, settings.provider),
        model: match settings.provider {
            Provider::Claude => settings.claude_model.clone(),
            Provider::OpenAi => settings.openai_model.clone(),
        },
        base_url: String::new(),
    };
    let cancel = jobs.start(&job_id);
    let emitter = app.clone();
    let id = job_id.clone();
    let result = blocking(move || {
        ai::run_task(
            &cfg,
            &task,
            &text,
            &mut |delta| {
                let _ = emitter.emit("ai-delta", AiDelta { job_id: id.clone(), delta: delta.to_string() });
            },
            &cancel,
        )
    })
    .await;
    jobs.finish(&job_id);
    result
}

// ---------- 문서 저장 ----------

/// 선택한 형식마다 파일을 만들고 저장된 경로를 돌려준다. 기존 파일은 덮어쓰지 않는다.
#[tauri::command]
async fn export_document(
    app: AppHandle,
    title: String,
    body: String,
    formats: Vec<Format>,
) -> CmdResult<Vec<String>> {
    if formats.is_empty() {
        return Err("저장할 형식을 하나 이상 선택해 주세요".into());
    }
    let settings = settings::load(&config_dir(&app)?);
    let dir = output_dir(&app, &settings)?;
    blocking(move || {
        std::fs::create_dir_all(&dir)
            .map_err(|e| Error::Export(format!("{} 폴더를 만들 수 없습니다: {e}", dir.display())))?;
        let doc = Document::new(title.clone(), body);
        let mut saved = Vec::new();
        for format in formats {
            let path = export::unique_path(&dir, &title, format);
            export::save(&doc, format, &path)?;
            saved.push(path.to_string_lossy().into_owned());
        }
        Ok(saved)
    })
    .await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Jobs::default())
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            set_api_key,
            list_models,
            download_model,
            delete_model,
            transcribe,
            format_transcript,
            cancel_job,
            run_ai,
            export_document,
        ])
        .run(tauri::generate_context!())
        .expect("앱을 시작하지 못했습니다");
}
