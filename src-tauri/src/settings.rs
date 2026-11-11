use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use voicedoc_core::ai::Provider;
use voicedoc_core::export::Format;
use voicedoc_core::stt::cloud;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SttMode {
    Local,
    Cloud,
}

/// 화면과 주고받는 설정. API 키는 여기에 넣지 않는다.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub stt_mode: SttMode,
    pub model_id: String,
    /// 빈 문자열이면 자동 감지.
    pub language: String,
    pub timestamps: bool,
    pub prompt: String,
    pub provider: Provider,
    pub claude_model: String,
    pub openai_model: String,
    pub cloud_stt_model: String,
    pub translate_target: String,
    /// 빈 문자열이면 문서 폴더 아래 VoiceDoc.
    pub output_dir: String,
    pub formats: Vec<Format>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            stt_mode: SttMode::Local,
            model_id: "large-v3-turbo-q5".into(),
            language: "ko".into(),
            timestamps: false,
            prompt: String::new(),
            provider: Provider::Claude,
            claude_model: Provider::Claude.default_model().into(),
            openai_model: Provider::OpenAi.default_model().into(),
            cloud_stt_model: cloud::DEFAULT_MODEL.into(),
            translate_target: "영어".into(),
            output_dir: String::new(),
            formats: vec![Format::Docx],
        }
    }
}

const FILE: &str = "settings.json";

pub fn load(dir: &Path) -> Settings {
    fs::read_to_string(dir.join(FILE))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(dir: &Path, settings: &Settings) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    fs::write(dir.join(FILE), json).map_err(|e| format!("설정을 저장하지 못했습니다: {e}"))
}
