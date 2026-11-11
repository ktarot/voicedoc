# VoiceDoc — 음성을 문서로

m4a · mp3 · mp4 · wav 등 음성(영상) 파일을 글로 바꿔 **docx · pdf · md · txt** 로 저장하는 앱입니다.

- **PC 단독 동작**: 음성 인식은 이 PC 안에서 처리합니다 (whisper.cpp). 인식 모델을 처음 한 번 내려받은 뒤에는 인터넷이 필요 없습니다.
- **온라인 AI (선택)**: 요약, 회의록 정리, 번역, 클라우드 고정밀 인식. 본인의 API 키를 설정에 넣었을 때만 동작합니다.
- **윈도 우선, 안드로이드 확장 대비**: Tauri 2 + Rust + React. 외부 프로그램(ffmpeg 등) 없이 모든 처리가 앱에 들어 있습니다.

## 사용 방법

1. 「파일 추가」를 누르거나 파일을 창에 끌어다 놓습니다.
2. 처음이라면 「모델 내려받기」를 누릅니다 (기본 모델 약 574MB, 한 번만).
3. 「변환 시작」을 누르면 차례로 글로 바뀝니다.
4. 결과를 확인·수정하고, 필요하면 「요약」「회의록」「번역」을 누릅니다.
5. 저장 형식을 고르고 「저장」을 누릅니다. 기본 저장 위치는 `문서\VoiceDoc` 입니다. 같은 이름의 파일은 덮어쓰지 않고 `(2)` 를 붙입니다.

### 온라인 AI 설정

「설정」에서 사용할 AI(Claude 또는 OpenAI)를 고르고 API 키를 저장합니다.

| 기능 | Claude | OpenAI |
|---|---|---|
| 요약 · 회의록 · 번역 | 가능 | 가능 |
| 클라우드 음성 인식 | 불가 (Claude API는 음성 입력을 받지 않음) | 가능 |

- API 키는 윈도 자격 증명 관리자에 보관되며 화면이나 설정 파일에 남지 않습니다.
- AI 기능을 쓸 때에만 녹취 내용(클라우드 인식은 음성)이 해당 회사 서버로 전송됩니다.
- 모델 이름은 설정에서 바꿀 수 있습니다. 기본값: Claude `claude-opus-5-5`, OpenAI `gpt-5`, 클라우드 인식 `whisper-1`.

## 개발

필요한 것: Node.js 20 이상, Rust(stable), CMake, LLVM(clang). 윈도에서는 Visual Studio C++ 빌드 도구와 WebView2(윈도 10/11에 기본 포함)도 필요합니다.

```bash
npm install
```

```bash
npm run tauri dev
```

```bash
npm run tauri build
```

윈도 설치 파일은 `target/release/bundle/nsis/` 에 만들어집니다. GitHub에 올렸다면 Actions의 「윈도 설치 파일 빌드」를 수동 실행하거나 `v*` 태그를 올려서 받을 수도 있습니다.

화면만 빠르게 볼 때는 `npm run dev` 후 브라우저로 `http://localhost:1420` 을 엽니다. 이때는 가짜 데이터로 동작합니다(`src/lib/mock.ts`).

### 테스트

```bash
cargo test -p voicedoc-core
```

형식별 디코딩(m4a, mp3, mp4, flac, ogg, wav), 네 가지 문서 저장, 온라인 AI 요청·응답 처리(가짜 서버)를 확인합니다.

### 명령줄 도구

```bash
cargo run -p voicedoc-core --bin voicedoc-cli -- 녹음.m4a --download --lang ko --formats docx,pdf
```

`--help` 로 전체 옵션을 볼 수 있습니다.

## 구조

```
crates/core/     핵심 라이브러리 (UI와 무관, 윈도·안드로이드 공용)
  audio.rs         디코딩 → 16kHz 모노 (symphonia, rubato)
  stt/local.rs     오프라인 인식 (whisper.cpp)
  stt/cloud.rs     클라우드 인식 (OpenAI, 10분 단위 분할 업로드)
  model.rs         인식 모델 목록·다운로드
  export/          txt, md, docx, pdf (한글 글꼴 내장)
  ai/              요약·회의록·번역 (Claude, OpenAI)
src-tauri/       Tauri 명령, 설정, API 키 보관
src/             React 화면
```

## 안드로이드 확장 시 할 일

핵심 라이브러리는 그대로 쓸 수 있도록 만들었습니다. 남은 작업은 다음과 같습니다.

- `npm run tauri android init` 후 NDK로 whisper.cpp 교차 컴파일 확인
- 파일 선택 결과가 경로가 아닌 content URI로 오므로, 앱 전용 폴더로 복사한 뒤 디코딩하도록 연결
- API 키 보관을 안드로이드 Keystore로 교체 (현재는 앱 전용 폴더의 파일)
- 저장 위치를 공유 저장소(문서 폴더)로 내보내는 처리
- 기본 모델을 base 또는 small 로 낮추기

## 라이선스 고지

- PDF에 내장되는 나눔고딕 글꼴: SIL Open Font License (`crates/core/assets/NanumGothic-OFL.txt`)
- whisper.cpp 및 Whisper 모델: MIT
