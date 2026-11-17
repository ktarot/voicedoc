//! 인식 결과에서 말이 아닌 것을 걷어 낸다: 소리 설명("(음악)"), 되풀이, 힌트 문장.

use crate::Segment;

fn normalize(text: &str) -> String {
    text.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// "(박수)", "[음악]", "♪♪", "*웃음*"처럼 말이 아닌 소리를 설명하는 표기만으로 된 글인지.
fn is_sound_note(text: &str) -> bool {
    let mut rest = text.trim();
    if rest.is_empty() {
        return true;
    }
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_whitespace() || "♪♫♬-–—.,·…".contains(c));
        let Some(open) = rest.chars().next() else { return true };
        let close = match open {
            '(' => ')',
            '[' => ']',
            '（' => '）',
            '【' => '】',
            '*' => '*',
            _ => return false,
        };
        match rest[open.len_utf8()..].find(close) {
            Some(i) => rest = &rest[open.len_utf8() + i + close.len_utf8()..],
            None => return false,
        }
    }
}

/// 문장 단위로 나눈다 (끝 문장부호는 문장에 붙여 둔다).
fn sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if matches!(c, '.' | '!' | '?' | '。' | '！' | '？') {
            // 이어지는 문장부호와 닫는 따옴표까지 포함
            let mut end = i + c.len_utf8();
            while let Some(&(j, n)) = chars.peek() {
                if matches!(n, '.' | '!' | '?' | '"' | '\'' | '”' | '’') {
                    end = j + n.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            // "3.5"처럼 숫자 사이의 점은 문장 끝이 아니다.
            let next_is_space = text[end..].chars().next().map_or(true, char::is_whitespace);
            if next_is_space {
                out.push(text[start..end].trim());
                start = end;
            }
        }
    }
    if !text[start..].trim().is_empty() {
        out.push(text[start..].trim());
    }
    out
}

/// 뜻이 있는 글자가 이만큼은 되어야 "되풀이"로 판단한다.
/// "네. 네." 같은 짧은 맞장구는 실제로 한 말일 수 있어 그대로 둔다.
const MIN_REPEAT_CHARS: usize = 6;

/// 앞 구간이 끝나고 이 시간 안에 시작한 구간만 되풀이인지 살핀다.
const REPEAT_WINDOW_MS: u64 = 1500;

/// 한 구간 안에서 같은 문장이 연달아 나오면 하나만 남긴다.
fn dedupe_sentences(text: &str, last: &mut String) -> String {
    let mut kept: Vec<&str> = Vec::new();
    for s in sentences(text) {
        let key = normalize(s);
        if key.chars().count() >= MIN_REPEAT_CHARS && key == *last {
            continue;
        }
        *last = key;
        kept.push(s);
    }
    kept.join(" ")
}

/// 인식 결과를 정리한다. `prompt`는 사용자가 넣은 인식 힌트.
pub fn clean(segments: Vec<Segment>, prompt: Option<&str>) -> Vec<Segment> {
    let prompt_key = prompt.map(normalize).filter(|p| p.chars().count() >= MIN_REPEAT_CHARS);
    let mut last = String::new();
    let mut last_end_ms = 0u64;
    let mut out: Vec<Segment> = Vec::with_capacity(segments.len());
    for mut seg in segments {
        if is_sound_note(&seg.text) {
            continue;
        }
        // 엔진이 헛도는 되풀이는 시간상 바로 이어진다. 한참 뒤에 같은 말을 다시 한 것은 실제 말이다.
        if seg.start_ms.saturating_sub(last_end_ms) >= REPEAT_WINDOW_MS {
            last.clear();
        }
        last_end_ms = seg.end_ms;
        // 소리가 불분명할 때 힌트 문장이 그대로 받아 적히는 경우
        if prompt_key.as_deref() == Some(normalize(&seg.text).as_str()) {
            continue;
        }
        seg.text = dedupe_sentences(&seg.text, &mut last);
        if seg.text.is_empty() {
            // 통째로 되풀이였던 구간: 앞 구간의 끝 시각만 늘려 둔다.
            continue;
        }
        out.push(seg);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(text: &str) -> Segment {
        Segment { start_ms: 0, end_ms: 1000, text: text.into() }
    }

    fn texts(input: &[&str], prompt: Option<&str>) -> Vec<String> {
        clean(input.iter().map(|t| seg(t)).collect(), prompt).into_iter().map(|s| s.text).collect()
    }

    #[test]
    fn drops_sound_notes() {
        assert_eq!(
            texts(&["(롤러스)", "[음악]", "♪♪", "*박수*", "(웃음) (박수)", "안녕하세요. (웃음)", "(주) 한국전자입니다."], None),
            vec!["안녕하세요. (웃음)", "(주) 한국전자입니다."]
        );
    }

    #[test]
    fn collapses_repeats_within_and_across_segments() {
        assert_eq!(
            texts(
                &[
                    "안녕하세요. 오늘 회의를 시작하겠습니다. 오늘 회의를 시작하겠습니다. 오늘 회의를 시작하겠습니다.",
                    "오늘 회의를 시작하겠습니다.",
                    "오늘 회의를 시작하겠습니다",
                    "첫 번째 안건입니다.",
                ],
                None
            ),
            vec!["안녕하세요. 오늘 회의를 시작하겠습니다.", "첫 번째 안건입니다."]
        );
    }

    #[test]
    fn keeps_short_repeats_and_separated_repeats() {
        assert_eq!(texts(&["네. 네.", "네.", "맞습니다."], None), vec!["네. 네.", "네.", "맞습니다."]);
        // 사이에 다른 말이 있으면 같은 문장이라도 지우지 않는다.
        assert_eq!(
            texts(&["다시 말씀드리겠습니다.", "일정은 3.5주입니다.", "다시 말씀드리겠습니다."], None).len(),
            3
        );
    }

    #[test]
    fn keeps_same_sentence_said_again_later() {
        let segs = vec![
            Segment { start_ms: 0, end_ms: 3000, text: "오늘 회의를 시작하겠습니다.".into() },
            Segment { start_ms: 3000, end_ms: 6000, text: "오늘 회의를 시작하겠습니다.".into() },
            Segment { start_ms: 40_000, end_ms: 43_000, text: "오늘 회의를 시작하겠습니다.".into() },
        ];
        let out = clean(segs, None);
        assert_eq!(out.iter().map(|s| s.start_ms).collect::<Vec<_>>(), vec![0, 40_000]);
    }

    #[test]
    fn drops_leaked_prompt_only() {
        let prompt = Some("홍길동 팀장, 분기 실적");
        assert_eq!(
            texts(&["홍길동 팀장 분기 실적", "홍길동 팀장이 분기 실적을 보고했습니다."], prompt),
            vec!["홍길동 팀장이 분기 실적을 보고했습니다."]
        );
    }

    #[test]
    fn splits_sentences() {
        assert_eq!(sentences("가격은 3.5달러입니다. 정말요?! 네"), vec!["가격은 3.5달러입니다.", "정말요?!", "네"]);
    }
}
