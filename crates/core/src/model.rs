//! 로컬 인식 모델(whisper.cpp ggml) 목록과 다운로드.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::{Cancel, Error, Result};

#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub file: &'static str,
    pub size_mb: u32,
    pub description: &'static str,
}

pub const MODELS: &[ModelInfo] = &[
    ModelInfo {
        id: "base",
        name: "기본 (base)",
        file: "ggml-base.bin",
        size_mb: 148,
        description: "가장 빠름. 또렷한 음성의 빠른 초안용",
    },
    ModelInfo {
        id: "small",
        name: "표준 (small)",
        file: "ggml-small.bin",
        size_mb: 488,
        description: "속도와 정확도의 균형. 일반 PC 권장",
    },
    ModelInfo {
        id: "large-v3-turbo-q5",
        name: "고품질 경량 (large-v3-turbo, 양자화)",
        file: "ggml-large-v3-turbo-q5_0.bin",
        size_mb: 574,
        description: "높은 정확도, 적은 용량. 한국어 권장",
    },
    ModelInfo {
        id: "medium",
        name: "정밀 (medium)",
        file: "ggml-medium.bin",
        size_mb: 1530,
        description: "정확도 높음, 느림",
    },
    ModelInfo {
        id: "large-v3-turbo",
        name: "고품질 (large-v3-turbo)",
        file: "ggml-large-v3-turbo.bin",
        size_mb: 1620,
        description: "가장 높은 정확도. 메모리 4GB 이상 필요",
    },
];

pub const DEFAULT_BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

pub fn find(id: &str) -> Option<&'static ModelInfo> {
    MODELS.iter().find(|m| m.id == id)
}

pub fn path_in(dir: &Path, info: &ModelInfo) -> PathBuf {
    dir.join(info.file)
}

pub fn is_downloaded(dir: &Path, info: &ModelInfo) -> bool {
    path_in(dir, info).is_file()
}

/// 모델을 내려받는다. 중간에 끊겨도 완성본만 남도록 임시 파일에 받은 뒤 이름을 바꾼다.
/// `progress(받은 바이트, 전체 바이트)`
pub fn download(
    dir: &Path,
    info: &ModelInfo,
    base_url: &str,
    mut progress: impl FnMut(u64, Option<u64>),
    cancel: &Cancel,
) -> Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let dest = path_in(dir, info);
    let part = dir.join(format!("{}.part", info.file));
    let url = format!("{}/{}", base_url.trim_end_matches('/'), info.file);

    let mut resp = crate::http_client(None)?
        .get(&url)
        .send()
        .map_err(|e| Error::Http(format!("모델 서버에 연결할 수 없습니다: {e}")))?;
    if !resp.status().is_success() {
        return Err(Error::Model(format!(
            "모델을 내려받지 못했습니다 (HTTP {})",
            resp.status().as_u16()
        )));
    }
    let total = resp.content_length();
    let result = (|| -> Result<u64> {
        let mut file = fs::File::create(&part)?;
        let mut buf = vec![0u8; 256 * 1024];
        let mut done: u64 = 0;
        loop {
            cancel.check()?;
            let n = resp.read(&mut buf).map_err(|e| Error::Http(e.to_string()))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])?;
            done += n as u64;
            progress(done, total);
        }
        file.flush()?;
        Ok(done)
    })();
    match result {
        Ok(done) if total.map(|t| t == done).unwrap_or(done > 0) => {
            fs::rename(&part, &dest)?;
            Ok(dest)
        }
        Ok(_) => {
            let _ = fs::remove_file(&part);
            Err(Error::Model("다운로드가 중간에 끊겼습니다. 다시 시도해 주세요".into()))
        }
        Err(e) => {
            let _ = fs::remove_file(&part);
            Err(e)
        }
    }
}
