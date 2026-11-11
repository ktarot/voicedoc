//! API 키 보관. 윈도·맥에서는 운영체제 보안 저장소를, 그 밖(리눅스, 안드로이드)에서는
//! 앱 전용 폴더의 파일을 쓴다. 키는 화면(웹뷰) 쪽으로 다시 내보내지 않는다.

use std::path::Path;

use voicedoc_core::ai::Provider;

fn account(provider: Provider) -> &'static str {
    match provider {
        Provider::Claude => "claude-api-key",
        Provider::OpenAi => "openai-api-key",
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
mod store {
    use super::*;

    const SERVICE: &str = "VoiceDoc";

    pub fn get(_dir: &Path, provider: Provider) -> String {
        keyring::Entry::new(SERVICE, account(provider))
            .and_then(|e| e.get_password())
            .unwrap_or_default()
    }

    pub fn set(_dir: &Path, provider: Provider, key: &str) -> Result<(), String> {
        let entry = keyring::Entry::new(SERVICE, account(provider)).map_err(|e| e.to_string())?;
        if key.is_empty() {
            match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(e) => Err(e.to_string()),
            }
        } else {
            entry.set_password(key).map_err(|e| e.to_string())
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod store {
    use super::*;
    use std::fs;

    fn path(dir: &Path, provider: Provider) -> std::path::PathBuf {
        dir.join(format!("{}.secret", account(provider)))
    }

    pub fn get(dir: &Path, provider: Provider) -> String {
        fs::read_to_string(path(dir, provider)).unwrap_or_default().trim().to_string()
    }

    pub fn set(dir: &Path, provider: Provider, key: &str) -> Result<(), String> {
        let p = path(dir, provider);
        if key.is_empty() {
            return match fs::remove_file(&p) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            };
        }
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        fs::write(&p, key).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&p, fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }
}

pub fn get(dir: &Path, provider: Provider) -> String {
    store::get(dir, provider)
}

pub fn set(dir: &Path, provider: Provider, key: &str) -> Result<(), String> {
    store::set(dir, provider, key.trim()).map_err(|e| format!("API 키를 저장하지 못했습니다: {e}"))
}
