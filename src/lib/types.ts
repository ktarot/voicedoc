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
  stage: "decode" | "transcribe";
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

export type ItemStatus = "queued" | "decoding" | "transcribing" | "done" | "error" | "cancelled";

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
