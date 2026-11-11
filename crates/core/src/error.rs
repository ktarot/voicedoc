use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

/// 사용자에게 그대로 보여 줄 수 있는 한국어 오류 메시지를 담는다.
#[derive(Debug, Error)]
pub enum Error {
    #[error("파일 오류: {0}")]
    Io(#[from] std::io::Error),
    #[error("오디오를 읽을 수 없습니다: {0}")]
    Decode(String),
    #[error("인식 모델 오류: {0}")]
    Model(String),
    #[error("음성 인식 오류: {0}")]
    Stt(String),
    #[error("네트워크 오류: {0}")]
    Http(String),
    #[error("AI 서비스 오류 ({status}): {message}")]
    Api { status: u16, message: String },
    #[error("AI가 이 요청을 처리하지 않았습니다: {0}")]
    Refused(String),
    #[error("문서 저장 오류: {0}")]
    Export(String),
    #[error("설정 오류: {0}")]
    Config(String),
    #[error("작업이 취소되었습니다")]
    Cancelled,
}
