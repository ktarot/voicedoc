//! 명령줄 도구. 화면 없이 인식·저장을 실행하거나 검증할 때 쓴다.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use voicedoc_core::ai::{self, AiConfig, Provider, Task};
use voicedoc_core::doc::Document;
use voicedoc_core::export::{self, Format};
use voicedoc_core::stt::{cloud, SttOptions};
use voicedoc_core::{audio, model, Cancel, Error, Result, Transcript};

const USAGE: &str = "\
사용법: voicedoc-cli <음성 파일> [옵션]

  --model <이름|경로>     로컬 모델 (base, small, large-v3-turbo-q5, medium, large-v3-turbo
                          또는 ggml 파일 경로). 기본값 large-v3-turbo-q5
  --models-dir <폴더>     모델 보관 폴더. 기본값 ./models
  --download              모델이 없으면 내려받는다
  --lang <코드>           언어 (ko, en …). 생략하면 자동 감지
  --prompt <문장>         고유명사 등 인식 힌트
  --cloud                 로컬 대신 OpenAI 클라우드 인식 (OPENAI_API_KEY 필요)
  --timestamps            문단마다 시각 표시
  --out <폴더>            저장 폴더. 기본값은 입력 파일과 같은 폴더
  --formats <목록>        저장 형식. 기본값 docx,md,txt,pdf
  --summary | --minutes | --translate <언어>
                          온라인 AI 후처리 결과를 별도 문서로 저장
  --provider <claude|openai>   AI 제공자. 기본값 claude
                          (ANTHROPIC_API_KEY 또는 OPENAI_API_KEY 필요)
  --ai-model <이름>       AI 모델 이름
";

struct Args {
    input: PathBuf,
    model: String,
    models_dir: PathBuf,
    download: bool,
    lang: Option<String>,
    prompt: Option<String>,
    cloud: bool,
    timestamps: bool,
    out: Option<PathBuf>,
    formats: Vec<Format>,
    task: Option<Task>,
    provider: Provider,
    ai_model: String,
}

fn parse_args() -> std::result::Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let mut a = Args {
        input: PathBuf::new(),
        model: "large-v3-turbo-q5".into(),
        models_dir: PathBuf::from("models"),
        download: false,
        lang: None,
        prompt: None,
        cloud: false,
        timestamps: false,
        out: None,
        formats: Format::ALL.to_vec(),
        task: None,
        provider: Provider::Claude,
        ai_model: String::new(),
    };
    let mut input = None;
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().ok_or(format!("{name} 뒤에 값이 필요합니다"));
        match arg.as_str() {
            "--model" => a.model = value("--model")?,
            "--models-dir" => a.models_dir = value("--models-dir")?.into(),
            "--download" => a.download = true,
            "--lang" => a.lang = Some(value("--lang")?),
            "--prompt" => a.prompt = Some(value("--prompt")?),
            "--cloud" => a.cloud = true,
            "--timestamps" => a.timestamps = true,
            "--out" => a.out = Some(value("--out")?.into()),
            "--formats" => {
                a.formats = value("--formats")?
                    .split(',')
                    .map(|f| Format::from_extension(f.trim()).ok_or(format!("알 수 없는 형식: {f}")))
                    .collect::<std::result::Result<_, _>>()?
            }
            "--summary" => a.task = Some(Task::Summary),
            "--minutes" => a.task = Some(Task::Minutes),
            "--translate" => a.task = Some(Task::Translate { target: value("--translate")? }),
            "--provider" => {
                a.provider = match value("--provider")?.as_str() {
                    "claude" => Provider::Claude,
                    "openai" => Provider::OpenAi,
                    other => return Err(format!("알 수 없는 제공자: {other}")),
                }
            }
            "--ai-model" => a.ai_model = value("--ai-model")?,
            "-h" | "--help" => return Err(String::new()),
            s if s.starts_with("--") => return Err(format!("알 수 없는 옵션: {s}")),
            _ if input.is_none() => input = Some(PathBuf::from(arg)),
            _ => return Err(format!("입력 파일은 하나만 지정할 수 있습니다: {arg}")),
        }
    }
    a.input = input.ok_or_else(String::new)?;
    Ok(a)
}

fn resolve_model(a: &Args, cancel: &Cancel) -> Result<PathBuf> {
    let as_path = Path::new(&a.model);
    if as_path.is_file() {
        return Ok(as_path.to_path_buf());
    }
    let info = model::find(&a.model)
        .ok_or_else(|| Error::Model(format!("'{}' 모델을 찾을 수 없습니다", a.model)))?;
    if model::is_downloaded(&a.models_dir, info) {
        return Ok(model::path_in(&a.models_dir, info));
    }
    if !a.download {
        return Err(Error::Model(format!(
            "{} 모델이 {} 에 없습니다. --download 옵션으로 내려받을 수 있습니다",
            info.id,
            a.models_dir.display()
        )));
    }
    eprintln!("모델 내려받는 중: {} ({}MB)", info.name, info.size_mb);
    let mut last = 0;
    model::download(
        &a.models_dir,
        info,
        model::DEFAULT_BASE_URL,
        |done, total| {
            let pct = total.map(|t| done * 100 / t.max(1)).unwrap_or(0);
            if pct >= last + 10 {
                last = pct;
                eprintln!("  {pct}%");
            }
        },
        cancel,
    )
}

fn transcribe(a: &Args, samples: &[f32], cancel: &Cancel) -> Result<Transcript> {
    let opts = SttOptions { language: a.lang.clone(), prompt: a.prompt.clone() };
    if a.cloud {
        let cfg = cloud::CloudSttConfig {
            api_key: std::env::var("OPENAI_API_KEY").unwrap_or_default(),
            model: cloud::DEFAULT_MODEL.into(),
            base_url: std::env::var("OPENAI_BASE_URL")
                .unwrap_or_else(|_| cloud::DEFAULT_BASE_URL.into()),
        };
        return cloud::transcribe(&cfg, samples, &opts, |p| eprintln!("  클라우드 인식 {p}%"), cancel);
    }
    #[cfg(feature = "local-stt")]
    {
        let model_path = resolve_model(a, cancel)?;
        let mut last = -10;
        voicedoc_core::stt::local::transcribe(
            &model_path,
            samples,
            &opts,
            move |p| {
                if p >= last + 10 {
                    last = p;
                    eprintln!("  인식 {p}%");
                }
            },
            cancel,
        )
    }
    #[cfg(not(feature = "local-stt"))]
    {
        let _ = resolve_model;
        Err(Error::Config("이 빌드에는 로컬 인식이 포함되어 있지 않습니다. --cloud 를 사용하세요".into()))
    }
}

fn run(a: &Args) -> Result<()> {
    let cancel = Cancel::new();
    let title = a
        .input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "문서".into());
    let out_dir = a
        .out
        .clone()
        .or_else(|| a.input.parent().map(Path::to_path_buf))
        .unwrap_or_default();

    eprintln!("오디오 읽는 중: {}", a.input.display());
    let samples = audio::decode_file(&a.input, |_| {}, &cancel)?;
    eprintln!("  길이 {:.1}초", samples.len() as f32 / audio::SAMPLE_RATE as f32);

    let transcript = transcribe(a, &samples, &cancel)?;
    eprintln!(
        "인식 완료: 구간 {}개, 언어 {}",
        transcript.segments.len(),
        transcript.language.as_deref().unwrap_or("?")
    );
    let body = transcript.to_text(a.timestamps);
    println!("{body}");

    let mut docs = vec![Document::new(title.clone(), body.clone())];
    if let Some(task) = &a.task {
        let (key_var, base_var) = match a.provider {
            Provider::Claude => ("ANTHROPIC_API_KEY", "ANTHROPIC_BASE_URL"),
            Provider::OpenAi => ("OPENAI_API_KEY", "OPENAI_BASE_URL"),
        };
        let cfg = AiConfig {
            provider: a.provider,
            api_key: std::env::var(key_var).unwrap_or_default(),
            model: a.ai_model.clone(),
            base_url: std::env::var(base_var).unwrap_or_default(),
        };
        eprintln!("AI 처리 중…");
        let result = ai::run_task(&cfg, task, &body, &mut |_| {}, &cancel)?;
        if result.truncated {
            eprintln!("주의: AI 출력이 길이 제한에 걸려 끝이 잘렸습니다");
        }
        println!("\n----\n{}", result.text);
        let suffix = match task {
            Task::Summary => "요약".to_string(),
            Task::Minutes => "회의록".to_string(),
            Task::Translate { target } => target.clone(),
        };
        docs.push(Document::new(format!("{title} - {suffix}"), result.text));
    }

    for doc in &docs {
        for &format in &a.formats {
            let path = out_dir.join(format!(
                "{}.{}",
                export::safe_file_stem(&doc.title),
                format.extension()
            ));
            export::save(doc, format, &path)?;
            eprintln!("저장: {}", path.display());
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(msg) => {
            if !msg.is_empty() {
                eprintln!("오류: {msg}\n");
            }
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("오류: {e}");
            ExitCode::FAILURE
        }
    }
}
