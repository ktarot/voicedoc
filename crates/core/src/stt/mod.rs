pub mod cloud;
#[cfg(feature = "local-stt")]
pub mod local;

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
}
