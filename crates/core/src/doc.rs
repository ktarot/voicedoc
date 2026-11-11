//! 내보내기용 문서 모델. 본문은 가벼운 마크다운(제목, 글머리표, 굵게)만 해석한다.

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Heading(u8, String),
    Bullet(String),
    Paragraph(String),
}

/// 굵게 여부가 붙은 글 조각.
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
}

#[derive(Debug, Clone)]
pub struct Document {
    pub title: String,
    /// 가벼운 마크다운 본문.
    pub body: String,
}

impl Document {
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self { title: title.into(), body: body.into() }
    }

    /// 제목을 포함한 블록 목록. 본문이 이미 큰 제목으로 시작하면 제목을 덧붙이지 않는다.
    pub fn blocks(&self) -> Vec<Block> {
        let mut blocks = parse_blocks(&self.body);
        let has_title = matches!(blocks.first(), Some(Block::Heading(1, _)));
        if !has_title && !self.title.trim().is_empty() {
            blocks.insert(0, Block::Heading(1, self.title.trim().to_string()));
        }
        blocks
    }
}

/// 비어 있지 않은 줄 하나가 블록 하나가 된다.
pub fn parse_blocks(text: &str) -> Vec<Block> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.chars().all(|c| c == '-' || c == '*' || c == '_') && line.len() >= 3 {
            continue;
        }
        let hashes = line.chars().take_while(|&c| c == '#').count();
        if (1..=6).contains(&hashes) && line[hashes..].starts_with(' ') {
            out.push(Block::Heading(hashes as u8, line[hashes..].trim().to_string()));
        } else if let Some(rest) = ["- ", "* ", "• "].iter().find_map(|p| line.strip_prefix(p)) {
            out.push(Block::Bullet(rest.trim().to_string()));
        } else {
            out.push(Block::Paragraph(line.to_string()));
        }
    }
    out
}

/// `**굵게**` 표시를 해석한다. 짝이 맞지 않는 `**`는 글자 그대로 둔다.
pub fn parse_spans(text: &str) -> Vec<Span> {
    let parts: Vec<&str> = text.split("**").collect();
    if parts.len() < 3 || parts.len() % 2 == 0 {
        return vec![Span { text: text.to_string(), bold: false }];
    }
    parts
        .into_iter()
        .enumerate()
        .filter(|(_, p)| !p.is_empty())
        .map(|(i, p)| Span { text: p.to_string(), bold: i % 2 == 1 })
        .collect()
}

/// 마크다운 강조 기호를 뺀 순수 텍스트.
pub fn plain(text: &str) -> String {
    parse_spans(text).into_iter().map(|s| s.text).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_blocks() {
        let b = parse_blocks("# 제목\n\n본문 한 줄\n- 항목 1\n* 항목 2\n---\n## 소제목\n#태그");
        assert_eq!(
            b,
            vec![
                Block::Heading(1, "제목".into()),
                Block::Paragraph("본문 한 줄".into()),
                Block::Bullet("항목 1".into()),
                Block::Bullet("항목 2".into()),
                Block::Heading(2, "소제목".into()),
                Block::Paragraph("#태그".into()),
            ]
        );
    }

    #[test]
    fn title_is_not_duplicated() {
        assert_eq!(Document::new("파일", "# 회의록\n내용").blocks().len(), 2);
        assert_eq!(Document::new("파일", "내용").blocks()[0], Block::Heading(1, "파일".into()));
    }

    #[test]
    fn parses_bold() {
        assert_eq!(
            parse_spans("가 **나** 다"),
            vec![
                Span { text: "가 ".into(), bold: false },
                Span { text: "나".into(), bold: true },
                Span { text: " 다".into(), bold: false },
            ]
        );
        assert_eq!(plain("2 ** 3"), "2 ** 3");
        assert_eq!(plain("**굵게**"), "굵게");
    }
}
