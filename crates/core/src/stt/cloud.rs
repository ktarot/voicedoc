//! OpenAI 전사 API를 이용한 클라우드 음성 인식.
//! 업로드 용량 제한(25MB)에 맞춰 조용한 지점에서 약 10분 단위로 잘라 보낸다.

use reqwest::blocking::multipart::{Form, Part};
use serde::Deserialize;

use super::SttOptions;
use crate::audio::{self, SAMPLE_RATE};
use crate::{Cancel, Error, Result, Segment, Transcript};

pub const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
pub const DEFAULT_MODEL: &str = "whisper-1";

const CHUNK_SAMPLES: usize = SAMPLE_RATE as usize * 600;
/// 자를 위치를 고를 때 뒤에서부터 살펴보는 범위.
const SEARCH_SAMPLES: usize = SAMPLE_RATE as usize * 15;
const FRAME: usize = SAMPLE_RATE as usize / 10;

#[derive(Debug, Clone)]
pub struct CloudSttConfig {
    pub api_key: String,
    pub model: String,
    pub base_url: String,
}

#[derive(Deserialize)]
struct ApiSegment {
    start: f64,
    end: f64,
    text: String,
}

#[derive(Deserialize)]
struct ApiResponse {
    #[serde(default)]
    text: String,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    segments: Vec<ApiSegment>,
}

/// 각 조각의 (시작, 끝) 샘플 위치. 경계는 가장 조용한 0.1초 구간에 둔다.
pub fn split_points(samples: &[f32]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    while samples.len() - start > CHUNK_SAMPLES {
        let hard_end = start + CHUNK_SAMPLES;
        let search_from = hard_end - SEARCH_SAMPLES;
        let mut best = hard_end;
        let mut best_energy = f32::MAX;
        let mut pos = search_from;
        while pos + FRAME <= hard_end {
            let energy: f32 = samples[pos..pos + FRAME].iter().map(|s| s * s).sum();
            if energy < best_energy {
                best_energy = energy;
                best = pos + FRAME / 2;
            }
            pos += FRAME;
        }
        out.push((start, best));
        start = best;
    }
    if start < samples.len() {
        out.push((start, samples.len()));
    }
    out
}

pub fn transcribe(
    cfg: &CloudSttConfig,
    samples: &[f32],
    opts: &SttOptions,
    mut progress: impl FnMut(i32),
    cancel: &Cancel,
) -> Result<Transcript> {
    if cfg.api_key.trim().is_empty() {
        return Err(Error::Config("클라우드 인식에는 OpenAI API 키가 필요합니다".into()));
    }
    let client = crate::http_client(Some(900))?;
    let url = format!("{}/audio/transcriptions", cfg.base_url.trim_end_matches('/'));
    // 구간별 시각 정보는 whisper-1의 verbose_json에서만 제공된다.
    let detailed = cfg.model == "whisper-1";
    let chunks = split_points(samples);
    let mut segments = Vec::new();
    let mut language = opts.language.clone();

    for (i, &(start, end)) in chunks.iter().enumerate() {
        cancel.check()?;
        progress((i * 100 / chunks.len()) as i32);
        let wav = audio::encode_wav(&samples[start..end]);
        let part = Part::bytes(wav)
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| Error::Http(e.to_string()))?;
        let mut form = Form::new()
            .part("file", part)
            .text("model", cfg.model.clone())
            .text("response_format", if detailed { "verbose_json" } else { "json" });
        if let Some(lang) = &opts.language {
            form = form.text("language", lang.clone());
        }
        if let Some(prompt) = opts.prompt.as_deref().filter(|p| !p.trim().is_empty()) {
            form = form.text("prompt", prompt.to_string());
        }
        let resp = client
            .post(&url)
            .bearer_auth(cfg.api_key.trim())
            .multipart(form)
            .send()
            .map_err(|e| Error::Http(e.to_string()))?;
        let status = resp.status();
        let body = resp.text().map_err(|e| Error::Http(e.to_string()))?;
        if !status.is_success() {
            return Err(crate::ai::api_error(status.as_u16(), &body));
        }
        let parsed: ApiResponse = serde_json::from_str(&body)
            .map_err(|e| Error::Stt(format!("응답을 해석할 수 없습니다: {e}")))?;
        if language.is_none() {
            language = parsed.language;
        }
        let offset_ms = audio::duration_ms(start);
        if parsed.segments.is_empty() {
            let text = parsed.text.trim();
            if !text.is_empty() {
                segments.push(Segment {
                    start_ms: offset_ms,
                    end_ms: audio::duration_ms(end),
                    text: text.to_string(),
                });
            }
        } else {
            for s in parsed.segments {
                let text = s.text.trim();
                if text.is_empty() {
                    continue;
                }
                segments.push(Segment {
                    start_ms: offset_ms + (s.start.max(0.0) * 1000.0) as u64,
                    end_ms: offset_ms + (s.end.max(0.0) * 1000.0) as u64,
                    text: text.to_string(),
                });
            }
        }
    }
    progress(100);
    Ok(Transcript { language, duration_ms: audio::duration_ms(samples.len()), segments })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_audio_is_one_chunk() {
        let s = vec![0.1f32; SAMPLE_RATE as usize * 30];
        assert_eq!(split_points(&s), vec![(0, s.len())]);
        assert!(split_points(&[]).is_empty());
    }

    #[test]
    fn long_audio_splits_at_silence() {
        let mut s = vec![0.5f32; SAMPLE_RATE as usize * 1300];
        // 9분 55초 지점에 0.5초 침묵
        let quiet = SAMPLE_RATE as usize * 595;
        for v in &mut s[quiet..quiet + SAMPLE_RATE as usize / 2] {
            *v = 0.0;
        }
        let parts = split_points(&s);
        assert_eq!(parts.len(), 3);
        assert!(parts[0].1 >= quiet && parts[0].1 <= quiet + SAMPLE_RATE as usize / 2);
        assert_eq!(parts.last().unwrap().1, s.len());
        for w in parts.windows(2) {
            assert_eq!(w[0].1, w[1].0);
            assert!(w[0].1 - w[0].0 <= CHUNK_SAMPLES);
        }
    }
}
