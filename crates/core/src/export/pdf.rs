//! PDF 저장. 한글이 깨지지 않도록 글꼴(나눔고딕, OFL)을 문서에 내장한다.

use std::io::{BufWriter, Cursor};

use printpdf::{IndirectFontRef, Mm, PdfDocument, PdfDocumentReference, PdfLayerReference};

use crate::doc::{plain, Block, Document};
use crate::{Error, Result};

static FONT: &[u8] = include_bytes!("../../assets/NanumGothic-Regular.ttf");

const PAGE_W: f32 = 210.0;
const PAGE_H: f32 = 297.0;
const MARGIN_X: f32 = 22.0;
const MARGIN_TOP: f32 = 24.0;
const MARGIN_BOTTOM: f32 = 22.0;
const PT_TO_MM: f32 = 0.352_778;
const LINE_HEIGHT: f32 = 1.6;
const BODY_PT: f32 = 10.5;
const BULLET_INDENT: f32 = 5.0;

struct Metrics<'a> {
    face: ttf_parser::Face<'a>,
    units: f32,
}

impl Metrics<'_> {
    /// 글자 폭(mm). 글꼴에 없는 글자는 전각 폭으로 본다.
    fn char_width(&self, c: char, pt: f32) -> f32 {
        let advance = self
            .face
            .glyph_index(c)
            .and_then(|g| self.face.glyph_hor_advance(g))
            .map(|a| a as f32 / self.units)
            .unwrap_or(1.0);
        advance * pt * PT_TO_MM
    }

    fn width(&self, s: &str, pt: f32) -> f32 {
        s.chars().map(|c| self.char_width(c, pt)).sum()
    }

    fn has_glyph(&self, c: char) -> bool {
        self.face.glyph_index(c).is_some()
    }

    /// 낱말 단위로 줄을 나누고, 한 줄보다 긴 낱말은 글자 단위로 나눈다.
    fn wrap(&self, text: &str, pt: f32, max_w: f32) -> Vec<String> {
        let mut lines = Vec::new();
        let mut line = String::new();
        let mut line_w = 0.0;
        let space_w = self.char_width(' ', pt);
        for word in text.split_whitespace() {
            let word_w = self.width(word, pt);
            let gap = if line.is_empty() { 0.0 } else { space_w };
            if line_w + gap + word_w <= max_w {
                if !line.is_empty() {
                    line.push(' ');
                }
                line.push_str(word);
                line_w += gap + word_w;
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
                line_w = 0.0;
            }
            if word_w <= max_w {
                line.push_str(word);
                line_w = word_w;
            } else {
                for c in word.chars() {
                    let cw = self.char_width(c, pt);
                    if line_w + cw > max_w && !line.is_empty() {
                        lines.push(std::mem::take(&mut line));
                        line_w = 0.0;
                    }
                    line.push(c);
                    line_w += cw;
                }
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
        lines
    }
}

struct Writer<'a> {
    pdf: PdfDocumentReference,
    font: IndirectFontRef,
    metrics: Metrics<'a>,
    layer: PdfLayerReference,
    /// 현재 줄 윗변의 위치 (종이 위쪽에서부터, mm)
    y: f32,
    page_no: u32,
}

impl Writer<'_> {
    fn footer(&self) {
        let label = format!("- {} -", self.page_no);
        let w = self.metrics.width(&label, 9.0);
        self.layer.use_text(label, 9.0, Mm((PAGE_W - w) / 2.0), Mm(12.0), &self.font);
    }

    fn new_page(&mut self) {
        self.footer();
        let (page, layer) = self.pdf.add_page(Mm(PAGE_W), Mm(PAGE_H), "본문");
        self.layer = self.pdf.get_page(page).get_layer(layer);
        self.y = MARGIN_TOP;
        self.page_no += 1;
    }

    fn ensure(&mut self, height: f32) {
        if self.y + height > PAGE_H - MARGIN_BOTTOM {
            self.new_page();
        }
    }

    fn space(&mut self, mm: f32) {
        // 쪽 맨 위에는 여백을 넣지 않는다.
        if self.y > MARGIN_TOP {
            self.y += mm;
        }
    }

    fn lines(&mut self, lines: &[String], pt: f32, x: f32, keep_with_next: f32) {
        let lh = pt * PT_TO_MM * LINE_HEIGHT;
        for (i, line) in lines.iter().enumerate() {
            self.ensure(lh + if i == 0 { keep_with_next } else { 0.0 });
            // 글자 기준선은 줄 윗변에서 글자 크기만큼 아래에 둔다.
            let baseline = PAGE_H - (self.y + pt * PT_TO_MM * 1.05);
            self.layer.use_text(line.as_str(), pt, Mm(x), Mm(baseline), &self.font);
            self.y += lh;
        }
    }
}

/// 글꼴에 없는 글자(이모지 등)는 PDF에서 빈칸으로 찍히므로 미리 걸러 낸다.
fn sanitize(text: &str, m: &Metrics) -> String {
    plain(text)
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|&c| c == ' ' || m.has_glyph(c))
        .collect()
}

/// 내장 글꼴과 본문 스트림을 압축해 파일 크기를 줄인다. 실패하면 원본을 그대로 쓴다.
/// 문서 정보의 제목도 한글이 깨지지 않도록 UTF-16으로 다시 적는다.
fn compress(raw: &[u8], title: &str) -> Option<Vec<u8>> {
    let mut pdf = lopdf::Document::load_mem(raw).ok()?;
    if let Ok(info_id) = pdf.trailer.get(b"Info").and_then(|o| o.as_reference()) {
        if let Ok(info) = pdf.get_object_mut(info_id).and_then(|o| o.as_dict_mut()) {
            let mut bytes = vec![0xFE, 0xFF];
            bytes.extend(title.encode_utf16().flat_map(u16::to_be_bytes));
            info.set("Title", lopdf::Object::String(bytes, lopdf::StringFormat::Hexadecimal));
        }
    }
    pdf.compress();
    let mut out = Vec::new();
    pdf.save_to(&mut out).ok()?;
    (out.len() < raw.len()).then_some(out)
}

pub fn render(doc: &Document) -> Result<Vec<u8>> {
    let face = ttf_parser::Face::parse(FONT, 0)
        .map_err(|e| Error::Export(format!("글꼴을 읽을 수 없습니다: {e}")))?;
    let metrics = Metrics { units: face.units_per_em() as f32, face };

    let title = if doc.title.trim().is_empty() { "문서" } else { doc.title.trim() };
    let (pdf, page, layer) = PdfDocument::new(title, Mm(PAGE_W), Mm(PAGE_H), "본문");
    let font = pdf
        .add_external_font(Cursor::new(FONT))
        .map_err(|e| Error::Export(format!("글꼴 내장 실패: {e}")))?;
    let layer = pdf.get_page(page).get_layer(layer);
    let mut w = Writer { pdf, font, metrics, layer, y: MARGIN_TOP, page_no: 1 };
    let text_w = PAGE_W - MARGIN_X * 2.0;
    let body_lh = BODY_PT * PT_TO_MM * LINE_HEIGHT;

    let mut prev_bullet = false;
    for block in doc.blocks() {
        let bullet = matches!(block, Block::Bullet(_));
        match block {
            Block::Heading(level, text) => {
                let (pt, before, after) = match level {
                    1 => (18.0, 0.0, 5.0),
                    2 => (13.5, 5.0, 2.0),
                    _ => (11.5, 3.5, 1.5),
                };
                let lines = w.metrics.wrap(&sanitize(&text, &w.metrics), pt, text_w);
                w.space(before);
                // 제목만 쪽 끝에 홀로 남지 않도록 본문 두 줄만큼 여유를 확인한다.
                w.lines(&lines, pt, MARGIN_X, after + body_lh * 2.0);
                w.y += after;
            }
            Block::Bullet(text) => {
                if !prev_bullet {
                    w.space(1.0);
                }
                let lines =
                    w.metrics.wrap(&sanitize(&text, &w.metrics), BODY_PT, text_w - BULLET_INDENT);
                if lines.is_empty() {
                    continue;
                }
                w.ensure(body_lh);
                let baseline = PAGE_H - (w.y + BODY_PT * PT_TO_MM * 1.05);
                w.layer.use_text("•", BODY_PT, Mm(MARGIN_X + 1.0), Mm(baseline), &w.font);
                w.lines(&lines, BODY_PT, MARGIN_X + BULLET_INDENT, 0.0);
                w.y += 0.8;
            }
            Block::Paragraph(text) => {
                let lines = w.metrics.wrap(&sanitize(&text, &w.metrics), BODY_PT, text_w);
                w.lines(&lines, BODY_PT, MARGIN_X, 0.0);
                w.y += 2.6;
            }
        }
        prev_bullet = bullet;
    }
    w.footer();

    let mut raw = Vec::new();
    w.pdf
        .save(&mut BufWriter::new(&mut raw))
        .map_err(|e| Error::Export(format!("PDF 생성 실패: {e}")))?;
    Ok(compress(&raw, title).unwrap_or(raw))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_within_width() {
        let face = ttf_parser::Face::parse(FONT, 0).unwrap();
        let m = Metrics { units: face.units_per_em() as f32, face };
        let text = "회의를 시작하겠습니다. ".repeat(30) + &"가".repeat(200);
        let lines = m.wrap(&text, BODY_PT, 166.0);
        assert!(lines.len() > 5);
        for l in &lines {
            assert!(m.width(l, BODY_PT) <= 166.0 + 0.01, "줄이 너무 깁니다: {l}");
        }
        let joined: String = lines.concat().split_whitespace().collect();
        let original: String = text.split_whitespace().collect();
        assert_eq!(joined, original);
    }
}
