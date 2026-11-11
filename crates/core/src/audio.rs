//! 음성·영상 파일을 인식용 PCM(16kHz, 모노, f32)으로 바꾼다.
//! 순수 Rust 디코더만 쓰므로 ffmpeg 같은 외부 프로그램이 필요 없다.

use std::fs::File;
use std::path::Path;

use rubato::{FftFixedIn, Resampler};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::{Cancel, Error, Result};

pub const SAMPLE_RATE: u32 = 16_000;

/// 지원하는 확장자 (파일 선택 창 필터용).
pub const EXTENSIONS: &[&str] = &[
    "m4a", "mp3", "mp4", "wav", "aac", "flac", "ogg", "oga", "m4b", "mov", "mkv", "webm", "3gp",
];

const RESAMPLE_CHUNK: usize = 2048;

/// 모노 샘플을 조금씩 받아 16kHz로 바꿔 쌓는다.
struct MonoResampler {
    inner: Option<FftFixedIn<f32>>,
    pending: Vec<f32>,
    out: Vec<f32>,
    skip: usize,
}

impl MonoResampler {
    fn new(rate: u32) -> Result<Self> {
        let inner = if rate == SAMPLE_RATE {
            None
        } else {
            Some(
                FftFixedIn::<f32>::new(rate as usize, SAMPLE_RATE as usize, RESAMPLE_CHUNK, 2, 1)
                    .map_err(|e| Error::Decode(format!("표본화 주파수 {rate}Hz 변환 실패: {e}")))?,
            )
        };
        let skip = inner.as_ref().map(|r| r.output_delay()).unwrap_or(0);
        Ok(Self { inner, pending: Vec::new(), out: Vec::new(), skip })
    }

    fn append(&mut self, produced: &[f32]) {
        let drop = self.skip.min(produced.len());
        self.skip -= drop;
        self.out.extend_from_slice(&produced[drop..]);
    }

    fn push(&mut self, mono: &[f32]) -> Result<()> {
        let Some(rs) = self.inner.as_mut() else {
            self.out.extend_from_slice(mono);
            return Ok(());
        };
        self.pending.extend_from_slice(mono);
        let mut produced = Vec::new();
        let mut offset = 0;
        while self.pending.len() - offset >= RESAMPLE_CHUNK {
            let res = rs
                .process(&[&self.pending[offset..offset + RESAMPLE_CHUNK]], None)
                .map_err(|e| Error::Decode(e.to_string()))?;
            produced.extend_from_slice(&res[0]);
            offset += RESAMPLE_CHUNK;
        }
        self.pending.drain(..offset);
        self.append(&produced);
        Ok(())
    }

    fn finish(mut self, input_frames: u64, rate: u32) -> Result<Vec<f32>> {
        if let Some(mut rs) = self.inner.take() {
            let expected = (input_frames as u128 * SAMPLE_RATE as u128 / rate as u128) as usize;
            // 남은 조각과 변환기 내부의 지연분을 무음으로 채워 밀어낸다.
            // (rubato의 process_partial은 남은 조각이 비어 있으면 실패하므로 직접 채운다.)
            let mut chunk = std::mem::take(&mut self.pending);
            let max_flushes = 4 + rs.output_delay() / (RESAMPLE_CHUNK * SAMPLE_RATE as usize / rate as usize).max(1);
            for _ in 0..max_flushes {
                if chunk.is_empty() && self.out.len() >= expected {
                    break;
                }
                chunk.resize(RESAMPLE_CHUNK, 0.0);
                let res = rs
                    .process(&[&chunk[..]], None)
                    .map_err(|e| Error::Decode(e.to_string()))?;
                self.append(&res[0].clone());
                chunk.clear();
            }
            self.out.truncate(expected);
        }
        Ok(self.out)
    }
}

/// 파일을 읽어 16kHz 모노 샘플로 돌려준다.
///
/// `progress`에는 0.0~1.0 사이 진행률이 전달된다 (길이를 알 수 없는 파일은 호출되지 않을 수 있다).
pub fn decode_file(
    path: &Path,
    mut progress: impl FnMut(f32),
    cancel: &Cancel,
) -> Result<Vec<f32>> {
    let file = File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(&ext.to_lowercase());
    }
    let probed = symphonia::default::get_probe()
        // 인코더가 앞뒤에 덧붙인 무음(지연)을 잘라 시각이 원본과 맞도록 한다.
        .format(
            &hint,
            mss,
            &FormatOptions { enable_gapless: true, ..Default::default() },
            &MetadataOptions::default(),
        )
        .map_err(|e| Error::Decode(format!("지원하지 않는 파일 형식입니다 ({e})")))?;
    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL && t.codec_params.sample_rate.is_some())
        .ok_or_else(|| Error::Decode("파일에 재생할 수 있는 음성 트랙이 없습니다".into()))?;
    let track_id = track.id;
    let total_frames = track.codec_params.n_frames;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| Error::Decode(format!("지원하지 않는 음성 코덱입니다 ({e})")))?;

    let mut resampler: Option<(MonoResampler, u32)> = None;
    let mut sample_buf: Option<SampleBuffer<f32>> = None;
    let mut mono: Vec<f32> = Vec::new();
    let mut frames_in: u64 = 0;
    let mut packets: u32 = 0;
    let mut decode_errors: u32 = 0;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymError::ResetRequired) => break,
            Err(e) => return Err(Error::Decode(e.to_string())),
        };
        if packet.track_id() != track_id {
            continue;
        }
        packets += 1;
        if packets % 64 == 0 {
            cancel.check()?;
            if let Some(total) = total_frames.filter(|&t| t > 0) {
                progress((frames_in as f32 / total as f32).min(1.0));
            }
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            // 손상된 패킷 하나 때문에 전체를 포기하지 않는다.
            Err(SymError::DecodeError(_)) => {
                decode_errors += 1;
                continue;
            }
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(Error::Decode(e.to_string())),
        };
        let spec = *decoded.spec();
        let channels = spec.channels.count().max(1);
        let frames = decoded.frames();
        if frames == 0 {
            continue;
        }
        let need_new_buf = sample_buf
            .as_ref()
            .map(|b| b.capacity() < frames * channels)
            .unwrap_or(true);
        if need_new_buf {
            sample_buf = Some(SampleBuffer::<f32>::new(decoded.capacity() as u64, spec));
        }
        let buf = sample_buf.as_mut().unwrap();
        buf.copy_interleaved_ref(decoded);
        let samples = buf.samples();

        mono.clear();
        mono.extend(
            samples
                .chunks_exact(channels)
                .map(|frame| frame.iter().sum::<f32>() / channels as f32),
        );
        frames_in += mono.len() as u64;

        if resampler.is_none() {
            resampler = Some((MonoResampler::new(spec.rate)?, spec.rate));
        }
        resampler.as_mut().unwrap().0.push(&mono)?;
    }

    let Some((rs, rate)) = resampler else {
        return Err(Error::Decode(if decode_errors > 0 {
            "음성 데이터를 해석하지 못했습니다 (손상되었거나 지원하지 않는 코덱)".into()
        } else {
            "파일에 음성 데이터가 없습니다".into()
        }));
    };
    let out = rs.finish(frames_in, rate)?;
    progress(1.0);
    Ok(out)
}

/// 16kHz 모노 샘플을 16비트 WAV 바이트로 만든다 (클라우드 인식 업로드용).
pub fn encode_wav(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // 모노
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

pub fn duration_ms(samples: usize) -> u64 {
    samples as u64 * 1000 / SAMPLE_RATE as u64
}
