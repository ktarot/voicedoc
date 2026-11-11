// 브라우저 미리보기용 흉내 백엔드. 실제 앱(Tauri)에서는 쓰이지 않는다.

import type { Backend } from "./api";
import type { AiDelta, JobProgress, ModelProgress, ModelView, Settings, Transcript } from "./types";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function emitter<T>() {
  const subs = new Set<(v: T) => void>();
  return {
    emit: (v: T) => subs.forEach((s) => s(v)),
    on: async (cb: (v: T) => void) => {
      subs.add(cb);
      return () => void subs.delete(cb);
    },
  };
}

const SAMPLE: Transcript = {
  language: "ko",
  duration_ms: 95_000,
  segments: [
    { start_ms: 0, end_ms: 4200, text: "안녕하세요. 주간 회의를 시작하겠습니다." },
    { start_ms: 4300, end_ms: 11_000, text: "오늘은 신제품 출시 일정과 마케팅 계획을 이야기하려고 합니다." },
    { start_ms: 14_000, end_ms: 22_000, text: "먼저 개발 쪽 진행 상황입니다. 베타 시험은 이번 달 말까지 마무리될 예정입니다." },
    { start_ms: 22_100, end_ms: 30_000, text: "발견된 문제는 우선순위에 따라 처리하고 있습니다." },
    { start_ms: 34_000, end_ms: 44_000, text: "마케팅은 사전 예약 행사를 2주 동안 진행하기로 했습니다. 예산은 지난 분기와 같습니다." },
    { start_ms: 48_000, end_ms: 58_000, text: "출시일은 3월 둘째 주 화요일로 확정하겠습니다. 김 팀장님이 금요일까지 일정표를 공유해 주세요." },
  ],
};

function toText(t: Transcript, timestamps: boolean): string {
  const paras: [number, string][] = [];
  let prevEnd = -1;
  for (const s of t.segments) {
    if (prevEnd < 0 || s.start_ms - prevEnd >= 2000) paras.push([s.start_ms, s.text]);
    else paras[paras.length - 1][1] += " " + s.text;
    prevEnd = s.end_ms;
  }
  const stamp = (ms: number) => {
    const s = Math.floor(ms / 1000);
    const p = (n: number) => String(n).padStart(2, "0");
    return `[${p(Math.floor(s / 3600))}:${p(Math.floor(s / 60) % 60)}:${p(s % 60)}] `;
  };
  return paras.map(([start, text]) => (timestamps ? stamp(start) : "") + text).join("\n\n");
}

const AI_TEXT: Record<string, string> = {
  summary:
    "# 요약\n신제품 출시 일정과 마케팅 계획을 논의했고, 출시일을 3월 둘째 주 화요일로 확정했다.\n\n## 주요 내용\n- 베타 시험은 이번 달 말까지 마무리\n- 사전 예약 행사를 2주간 진행, 예산은 지난 분기와 동일\n- 김 팀장이 금요일까지 일정표 공유",
  minutes:
    "# 회의록\n\n## 개요\n신제품 출시 일정과 마케팅 계획 논의\n\n## 논의 내용\n- 개발: 베타 시험은 이번 달 말 마무리 예정\n- 마케팅: 사전 예약 행사 2주 진행\n\n## 결정 사항\n- 출시일은 **3월 둘째 주 화요일**\n\n## 할 일\n- 김 팀장: 일정표 공유 (금요일까지)",
  translate:
    "Hello. Let's begin the weekly meeting. Today we will talk about the launch schedule for the new product and the marketing plan.\n\nFirst, the development status. Beta testing is expected to wrap up by the end of this month.",
};

export function createMockBackend(): Backend {
  let settings: Settings = {
    sttMode: "local",
    modelId: "large-v3-turbo-q5",
    language: "ko",
    timestamps: false,
    prompt: "",
    provider: "claude",
    claudeModel: "claude-opus-5-5",
    openaiModel: "gpt-5",
    cloudSttModel: "whisper-1",
    translateTarget: "영어",
    outputDir: "",
    formats: ["docx"],
  };
  const keys = { claude: false, openai: false };
  const models: ModelView[] = [
    { id: "base", name: "기본 (base)", sizeMb: 148, description: "가장 빠름. 또렷한 음성의 빠른 초안용", downloaded: true },
    { id: "small", name: "표준 (small)", sizeMb: 488, description: "속도와 정확도의 균형. 일반 PC 권장", downloaded: false },
    { id: "large-v3-turbo-q5", name: "고품질 경량 (large-v3-turbo, 양자화)", sizeMb: 574, description: "높은 정확도, 적은 용량. 한국어 권장", downloaded: false },
  ];
  const cancelled = new Set<string>();
  const jobProgress = emitter<JobProgress>();
  const modelProgress = emitter<ModelProgress>();
  const aiDelta = emitter<AiDelta>();
  const check = (id: string) => {
    if (cancelled.delete(id)) throw "작업이 취소되었습니다";
  };

  return {
    getSettings: async () => ({
      ...settings,
      hasClaudeKey: keys.claude,
      hasOpenaiKey: keys.openai,
      resolvedOutputDir: settings.outputDir || "C:\\Users\\me\\Documents\\VoiceDoc",
      audioExtensions: ["m4a", "mp3", "mp4", "wav"],
    }),
    saveSettings: async (s) => void (settings = s),
    setApiKey: async (provider, key) => void (keys[provider] = key.trim() !== ""),
    listModels: async () => models.map((m) => ({ ...m })),
    async downloadModel(id) {
      const m = models.find((x) => x.id === id)!;
      const total = m.sizeMb * 1024 * 1024;
      for (let i = 1; i <= 20; i++) {
        await sleep(120);
        check(`model:${id}`);
        modelProgress.emit({ id, done: (total * i) / 20, total });
      }
      m.downloaded = true;
    },
    deleteModel: async (id) => void (models.find((x) => x.id === id)!.downloaded = false),
    async transcribe(jobId, path) {
      if (path.includes("오류")) throw "오디오를 읽을 수 없습니다: 지원하지 않는 파일 형식입니다";
      for (const stage of ["decode", "transcribe"] as const) {
        for (let p = 0; p <= 100; p += stage === "decode" ? 25 : 5) {
          await sleep(60);
          check(jobId);
          jobProgress.emit({ jobId, stage, percent: p });
        }
      }
      return { transcript: SAMPLE, text: toText(SAMPLE, settings.timestamps) };
    },
    formatTranscript: async (t, timestamps) => toText(t, timestamps),
    cancelJob: async (jobId) => void cancelled.add(jobId),
    async runAi(jobId, task) {
      if (!keys[settings.provider]) throw "설정 오류: AI 기능을 쓰려면 설정에서 API 키를 입력해 주세요";
      const text = AI_TEXT[task.kind];
      for (let i = 0; i < text.length; i += 6) {
        await sleep(25);
        check(jobId);
        aiDelta.emit({ jobId, delta: text.slice(i, i + 6) });
      }
      return { text, truncated: false };
    },
    exportDocument: async (title, _body, formats) => {
      await sleep(300);
      return formats.map((f) => `C:\\Users\\me\\Documents\\VoiceDoc\\${title}.${f}`);
    },
    pickFiles: async () => ["C:\\녹음\\주간 회의 2026-11-10.m4a", "C:\\녹음\\고객 인터뷰.mp3"],
    pickFolder: async () => "D:\\문서\\회의록",
    reveal: async () => {},
    onJobProgress: jobProgress.on,
    onModelProgress: modelProgress.on,
    onAiDelta: aiDelta.on,
    onFileDrop: async () => () => {},
  };
}
