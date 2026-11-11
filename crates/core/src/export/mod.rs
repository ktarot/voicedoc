//! 문서 저장: txt, md, docx, pdf.

mod docx;
mod pdf;
mod text;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::doc::Document;
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Txt,
    Md,
    Docx,
    Pdf,
}

impl Format {
    pub const ALL: [Format; 4] = [Format::Docx, Format::Md, Format::Txt, Format::Pdf];

    pub fn extension(self) -> &'static str {
        match self {
            Format::Txt => "txt",
            Format::Md => "md",
            Format::Docx => "docx",
            Format::Pdf => "pdf",
        }
    }

    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.trim_start_matches('.').to_lowercase().as_str() {
            "txt" => Some(Format::Txt),
            "md" | "markdown" => Some(Format::Md),
            "docx" => Some(Format::Docx),
            "pdf" => Some(Format::Pdf),
            _ => None,
        }
    }
}

/// 문서를 바이트로 만든다.
pub fn render(doc: &Document, format: Format) -> Result<Vec<u8>> {
    match format {
        Format::Txt => Ok(text::render_txt(doc).into_bytes()),
        Format::Md => Ok(text::render_md(doc).into_bytes()),
        Format::Docx => docx::render(doc),
        Format::Pdf => pdf::render(doc),
    }
}

/// 문서를 파일로 저장한다.
pub fn save(doc: &Document, format: Format, path: &Path) -> Result<()> {
    let bytes = render(doc, format)?;
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes).map_err(|e| {
        Error::Export(format!("{} 에 저장할 수 없습니다: {e}", path.display()))
    })
}

/// 파일 이름으로 쓸 수 없는 문자를 바꾼다.
pub fn safe_file_stem(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| if c.is_control() || r#"\/:*?"<>|"#.contains(c) { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches('.').to_string();
    if cleaned.is_empty() { "문서".into() } else { cleaned }
}

/// `dir/제목.확장자` 경로. 같은 이름이 있으면 ` (2)`, ` (3)`… 을 붙인다.
pub fn unique_path(dir: &Path, title: &str, format: Format) -> PathBuf {
    let stem = safe_file_stem(title);
    let mut path = dir.join(format!("{stem}.{}", format.extension()));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem} ({n}).{}", format.extension()));
        n += 1;
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names() {
        assert_eq!(safe_file_stem("회의: 3/4분기?"), "회의_ 3_4분기_");
        assert_eq!(safe_file_stem("  "), "문서");
        assert_eq!(Format::from_extension(".PDF"), Some(Format::Pdf));
    }
}
