use crate::doc::{parse_blocks, Block, Document};

fn starts_with_title(doc: &Document) -> bool {
    matches!(parse_blocks(&doc.body).first(), Some(Block::Heading(1, _)))
}

pub fn render_md(doc: &Document) -> String {
    let body = doc.body.trim();
    if starts_with_title(doc) || doc.title.trim().is_empty() {
        format!("{body}\n")
    } else {
        format!("# {}\n\n{body}\n", doc.title.trim())
    }
}

/// 일반 텍스트: 마크다운 기호를 걷어 내고 읽기 좋게 줄을 나눈다.
pub fn render_txt(doc: &Document) -> String {
    let mut out = String::new();
    let mut prev_bullet = false;
    for block in doc.blocks() {
        let bullet = matches!(block, Block::Bullet(_));
        if !out.is_empty() && !(bullet && prev_bullet) {
            out.push('\n');
        }
        match block {
            Block::Heading(level, text) => {
                let text = crate::doc::plain(&text);
                out.push_str(&text);
                out.push('\n');
                if level == 1 {
                    out.push_str(&"=".repeat(text.chars().count().clamp(4, 40)));
                    out.push('\n');
                }
            }
            Block::Bullet(text) => {
                out.push_str("- ");
                out.push_str(&crate::doc::plain(&text));
                out.push('\n');
            }
            Block::Paragraph(text) => {
                out.push_str(&crate::doc::plain(&text));
                out.push('\n');
            }
        }
        prev_bullet = bullet;
    }
    // 윈도 메모장 등에서도 줄바꿈이 보이도록 CRLF로 저장한다.
    out.replace('\n', "\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md_adds_title_once() {
        assert_eq!(render_md(&Document::new("녹음", "본문")), "# 녹음\n\n본문\n");
        assert_eq!(render_md(&Document::new("녹음", "# 회의록\n본문")), "# 회의록\n본문\n");
    }

    #[test]
    fn txt_strips_markdown() {
        let t = render_txt(&Document::new("녹음", "## 결정\n- **A안** 채택\n- 다음 주 공유\n\n끝."));
        assert_eq!(t, "녹음\r\n====\r\n\r\n결정\r\n\r\n- A안 채택\r\n- 다음 주 공유\r\n\r\n끝.\r\n");
    }
}
