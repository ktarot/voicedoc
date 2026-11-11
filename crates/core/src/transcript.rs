use serde::{Deserialize, Serialize};

/// 인식된 한 구간.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Segment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Transcript {
    /// 인식된 언어 코드 (예: "ko"). 알 수 없으면 None.
    pub language: Option<String>,
    pub duration_ms: u64,
    pub segments: Vec<Segment>,
}

/// 이보다 긴 침묵이 있으면 문단을 나눈다.
const PARAGRAPH_GAP_MS: u64 = 2000;
/// 문단이 이 길이를 넘으면 문장이 끝나는 곳에서 나눈다.
const PARAGRAPH_SOFT_LIMIT: usize = 400;

pub fn format_timestamp(ms: u64) -> String {
    let s = ms / 1000;
    format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

fn ends_sentence(text: &str) -> bool {
    matches!(
        text.trim_end().chars().last(),
        Some('.' | '!' | '?' | '。' | '？' | '！' | '…')
    )
}

impl Transcript {
    /// 구간들을 읽기 좋은 문단으로 묶는다. (문단 시작 시각, 문단 텍스트)
    pub fn paragraphs(&self) -> Vec<(u64, String)> {
        let mut out: Vec<(u64, String)> = Vec::new();
        let mut prev_end: Option<u64> = None;
        let mut prev_ended_sentence = false;
        for seg in &self.segments {
            let text = seg.text.trim();
            if text.is_empty() {
                continue;
            }
            let start_new = match (out.last(), prev_end) {
                (Some((_, cur)), Some(end)) => {
                    seg.start_ms.saturating_sub(end) >= PARAGRAPH_GAP_MS
                        || (cur.chars().count() >= PARAGRAPH_SOFT_LIMIT && prev_ended_sentence)
                }
                _ => true,
            };
            if start_new {
                out.push((seg.start_ms, text.to_string()));
            } else if let Some((_, cur)) = out.last_mut() {
                cur.push(' ');
                cur.push_str(text);
            }
            prev_end = Some(seg.end_ms);
            prev_ended_sentence = ends_sentence(text);
        }
        out
    }

    /// 문서 본문으로 쓸 텍스트. 문단 사이는 빈 줄로 구분한다.
    pub fn to_text(&self, timestamps: bool) -> String {
        self.paragraphs()
            .into_iter()
            .map(|(start, text)| {
                if timestamps {
                    format!("[{}] {}", format_timestamp(start), text)
                } else {
                    text
                }
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start: u64, end: u64, text: &str) -> Segment {
        Segment { start_ms: start, end_ms: end, text: text.into() }
    }

    #[test]
    fn groups_by_silence() {
        let t = Transcript {
            language: None,
            duration_ms: 20_000,
            segments: vec![
                seg(0, 3000, " 안녕하세요."),
                seg(3200, 6000, "회의를 시작하겠습니다."),
                seg(9000, 12_000, "첫 번째 안건입니다."),
                seg(12_000, 12_500, "  "),
            ],
        };
        assert_eq!(
            t.to_text(false),
            "안녕하세요. 회의를 시작하겠습니다.\n\n첫 번째 안건입니다."
        );
        assert_eq!(
            t.to_text(true),
            "[00:00:00] 안녕하세요. 회의를 시작하겠습니다.\n\n[00:00:09] 첫 번째 안건입니다."
        );
    }

    #[test]
    fn splits_long_paragraph_at_sentence_end() {
        let long = "가".repeat(450) + ".";
        let t = Transcript {
            language: None,
            duration_ms: 0,
            segments: vec![seg(0, 1000, &long), seg(1000, 2000, "다음 문장.")],
        };
        assert_eq!(t.paragraphs().len(), 2);
    }

    #[test]
    fn timestamp_format() {
        assert_eq!(format_timestamp(3_723_000), "01:02:03");
    }
}
