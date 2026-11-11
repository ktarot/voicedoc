//! VoiceDoc 핵심 라이브러리.
//!
//! Tauri 등 UI 계층에 의존하지 않으며 윈도·안드로이드에서 같은 코드를 쓴다.
//! 모든 함수는 블로킹 방식이므로 UI 스레드가 아닌 작업 스레드에서 호출한다.

pub mod ai;
pub mod audio;
pub mod doc;
pub mod error;
pub mod export;
pub mod model;
pub mod stt;
pub mod transcript;

pub use error::{Error, Result};
pub use transcript::{Segment, Transcript};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// 긴 작업을 중간에 멈추기 위한 취소 신호.
#[derive(Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
    #[allow(dead_code)]
    pub(crate) fn flag(&self) -> &AtomicBool {
        &self.0
    }
    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}

pub(crate) fn http_client(timeout_secs: Option<u64>) -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .timeout(timeout_secs.map(std::time::Duration::from_secs))
        .user_agent(concat!("voicedoc/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| Error::Http(e.to_string()))
}
