use docx_rs::{Docx, LineSpacing, Paragraph, Run, RunFonts, SpecialIndentType};

use crate::doc::{parse_spans, Block, Document};
use crate::{Error, Result};

/// 윈도에 기본 설치된 한글 글꼴. 없는 환경에서는 워드가 대체 글꼴을 고른다.
const FONT: &str = "맑은 고딕";

fn fonts() -> RunFonts {
    RunFonts::new().ascii(FONT).hi_ansi(FONT).east_asia(FONT).cs(FONT)
}

/// `size`는 반 포인트 단위 (21 = 10.5pt).
fn runs(mut p: Paragraph, text: &str, size: usize, force_bold: bool) -> Paragraph {
    for span in parse_spans(text) {
        let mut run = Run::new().add_text(span.text).size(size).fonts(fonts());
        if span.bold || force_bold {
            run = run.bold();
        }
        p = p.add_run(run);
    }
    p
}

pub fn render(doc: &Document) -> Result<Vec<u8>> {
    let mut docx = Docx::new().default_fonts(fonts()).default_size(21);
    for block in doc.blocks() {
        let p = match block {
            Block::Heading(level, text) => {
                let (size, before, after) = match level {
                    1 => (36, 0, 240),
                    2 => (28, 320, 120),
                    _ => (24, 240, 80),
                };
                runs(Paragraph::new(), &text, size, true)
                    .line_spacing(LineSpacing::new().before(before).after(after))
                    .keep_next(true)
            }
            Block::Bullet(text) => runs(Paragraph::new(), &format!("•  {text}"), 21, false)
                .indent(Some(420), Some(SpecialIndentType::Hanging(280)), None, None)
                .line_spacing(LineSpacing::new().after(60).line(340)),
            Block::Paragraph(text) => runs(Paragraph::new(), &text, 21, false)
                .line_spacing(LineSpacing::new().after(160).line(340)),
        };
        docx = docx.add_paragraph(p);
    }
    let mut out = std::io::Cursor::new(Vec::new());
    docx.build()
        .pack(&mut out)
        .map_err(|e| Error::Export(format!("DOCX 생성 실패: {e}")))?;
    Ok(out.into_inner())
}
