export type Format = "docx" | "md" | "txt" | "pdf";
export type Provider = "claude" | "openai";
export type SttMode = "local" | "cloud";

export interface Settings {
  sttMode: SttMode;
  modelId: string;
  /** 빈 문자열이면 자동 감지 */
  language: string;
  timestamps: boolean;
  prompt: string;
  /** 그래픽카드가 있으면 인식에 쓴다 */
  useGpu: boolean;
  /** 침묵·잡음 구간을 건너뛰지 않고 전체를 인식 */
  keepSilence: boolean;
  provider: Provider;
  claudeModel: string;
  openaiModel: string;
  cloudSttModel: string;
  translateTarget: string;
  outputDir: string;
  formats: Format[];
}

export interface SettingsView extends Settings {
  hasClaudeKey: boolean;
  hasOpenaiKey: boolean;
  resolvedOutputDir: string;
  audioExtensions: string[];
  /** 지난 실행에서 그래픽카드 인식 중 앱이 종료되어 CPU 인식으로 바꿨다 */
  gpuAutoDisabled: boolean;
}

export interface ModelView {
  id: string;
  name: string;
  sizeMb: number;
  description: string;
  downloaded: boolean;
}

export interface Segment {
  start_ms: number;
  end_ms: number;
  text: string;
}

export interface Transcript {
  language: string | null;
  duration_ms: number;
  segments: Segment[];
  /** 말소리가 감지됐지만 받아 적지 못한 구간 [시작 ms, 끝 ms] */
  unrecognized?: [number, number][];
  /** 인식은 끝났지만 알려야 할 문제 */
  warnings?: string[];
}

export interface TranscribeResult {
  transcript: Transcript;
  text: string;
}

export type AiTask =
  | { kind: "summary" }
  | { kind: "minutes" }
  | { kind: "translate"; target: string };

export interface AiOutput {
  text: string;
  truncated: boolean;
}

export interface JobProgress {
  jobId: string;
  stage: "decode" | "scan" | "load" | "transcribe";
  percent: number;
}

export interface ModelProgress {
  id: string;
  done: number;
  total: number | null;
}

export interface AiDelta {
  jobId: string;
  delta: string;
}

export type DocKind = "original" | "summary" | "minutes" | "translation";

export type ItemStatus =
  | "queued"
  | "decoding"
  | "scanning"
  | "loading"
  | "transcribing"
  | "done"
  | "error"
  | "cancelled";

const BUSY: ItemStatus[] = ["decoding", "scanning", "loading", "transcribing"];
export const isBusy = (status: ItemStatus) => BUSY.includes(status);

export const STAGE_STATUS: Record<JobProgress["stage"], ItemStatus> = {
  decode: "decoding",
  scan: "scanning",
  load: "loading",
  transcribe: "transcribing",
};

/** 진행 중일 때 큰 화면에 보여 줄 설명 */
export const BUSY_MESSAGE: Partial<Record<ItemStatus, string>> = {
  decoding: "오디오를 읽고 있습니다…",
  scanning: "말소리가 있는 구간을 찾고 있습니다…",
  loading: "인식 모델을 불러오고 있습니다… (처음 한 번은 수십 초가 걸릴 수 있습니다)",
  transcribing: "음성을 글로 바꾸고 있습니다…",
};

export interface Item {
  id: string;
  path: string;
  name: string;
  status: ItemStatus;
  percent: number;
  error?: string;
  transcript?: Transcript;
  /** 원문을 손으로 고쳤는지 여부 */
  edited: boolean;
  docs: Partial<Record<DocKind, string>>;
}

export const DOC_LABELS: Record<DocKind, string> = {
  original: "원문",
  summary: "요약",
  minutes: "회의록",
  translation: "번역",
};

export const FORMAT_LABELS: Record<Format, string> = {
  docx: "워드 (.docx)",
  pdf: "PDF (.pdf)",
  md: "마크다운 (.md)",
  txt: "텍스트 (.txt)",
};

export const LANGUAGES: [code: string, label: string][] = [
  ["", "자동 감지"],
  ["ko", "한국어"],
  ["en", "영어"],
  ["ja", "일본어"],
  ["zh", "중국어"],
  ["es", "스페인어"],
  ["fr", "프랑스어"],
  ["de", "독일어"],
  ["vi", "베트남어"],
  ["ru", "러시아어"],
];

export const TRANSLATE_TARGETS = ["영어", "한국어", "일본어", "중국어", "스페인어", "프랑스어", "독일어", "베트남어"];
