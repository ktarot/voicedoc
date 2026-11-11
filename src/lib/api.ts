// 화면과 백엔드 사이의 단일 창구.
// Tauri 안에서는 실제 명령을 호출하고, 일반 브라우저(개발 미리보기)에서는 흉내 구현을 쓴다.

import type {
  AiDelta,
  AiOutput,
  AiTask,
  Format,
  JobProgress,
  ModelProgress,
  ModelView,
  Provider,
  Settings,
  SettingsView,
  TranscribeResult,
  Transcript,
} from "./types";

export type Unlisten = () => void;

export interface Backend {
  getSettings(): Promise<SettingsView>;
  saveSettings(settings: Settings): Promise<void>;
  setApiKey(provider: Provider, key: string): Promise<void>;
  listModels(): Promise<ModelView[]>;
  downloadModel(id: string): Promise<void>;
  deleteModel(id: string): Promise<void>;
  transcribe(jobId: string, path: string): Promise<TranscribeResult>;
  formatTranscript(transcript: Transcript, timestamps: boolean): Promise<string>;
  cancelJob(jobId: string): Promise<void>;
  runAi(jobId: string, task: AiTask, text: string): Promise<AiOutput>;
  exportDocument(title: string, body: string, formats: Format[]): Promise<string[]>;
  pickFiles(extensions: string[]): Promise<string[]>;
  pickFolder(): Promise<string | null>;
  reveal(path: string): Promise<void>;
  onJobProgress(cb: (p: JobProgress) => void): Promise<Unlisten>;
  onModelProgress(cb: (p: ModelProgress) => void): Promise<Unlisten>;
  onAiDelta(cb: (d: AiDelta) => void): Promise<Unlisten>;
  onFileDrop(cb: (paths: string[]) => void, onHover: (over: boolean) => void): Promise<Unlisten>;
}

const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function createTauriBackend(): Promise<Backend> {
  const { invoke } = await import("@tauri-apps/api/core");
  const { listen } = await import("@tauri-apps/api/event");
  const { getCurrentWebview } = await import("@tauri-apps/api/webview");
  const dialog = await import("@tauri-apps/plugin-dialog");
  const opener = await import("@tauri-apps/plugin-opener");

  return {
    getSettings: () => invoke("get_settings"),
    saveSettings: (settings) => invoke("save_settings", { settings }),
    setApiKey: (provider, key) => invoke("set_api_key", { provider, key }),
    listModels: () => invoke("list_models"),
    downloadModel: (id) => invoke("download_model", { id }),
    deleteModel: (id) => invoke("delete_model", { id }),
    transcribe: (jobId, path) => invoke("transcribe", { jobId, path }),
    formatTranscript: (transcript, timestamps) => invoke("format_transcript", { transcript, timestamps }),
    cancelJob: (jobId) => invoke("cancel_job", { jobId }),
    runAi: (jobId, task, text) => invoke("run_ai", { jobId, task, text }),
    exportDocument: (title, body, formats) => invoke("export_document", { title, body, formats }),
    async pickFiles(extensions) {
      const picked = await dialog.open({
        multiple: true,
        filters: [{ name: "음성·영상 파일", extensions }],
      });
      if (!picked) return [];
      return Array.isArray(picked) ? picked : [picked];
    },
    async pickFolder() {
      const picked = await dialog.open({ directory: true });
      return typeof picked === "string" ? picked : null;
    },
    reveal: (path) => opener.revealItemInDir(path),
    onJobProgress: (cb) => listen<JobProgress>("job-progress", (e) => cb(e.payload)),
    onModelProgress: (cb) => listen<ModelProgress>("model-progress", (e) => cb(e.payload)),
    onAiDelta: (cb) => listen<AiDelta>("ai-delta", (e) => cb(e.payload)),
    onFileDrop: (cb, onHover) =>
      getCurrentWebview().onDragDropEvent((e) => {
        if (e.payload.type === "over" || e.payload.type === "enter") onHover(true);
        else if (e.payload.type === "leave") onHover(false);
        else if (e.payload.type === "drop") {
          onHover(false);
          cb(e.payload.paths);
        }
      }),
  };
}

let backend: Promise<Backend> | undefined;

export function api(): Promise<Backend> {
  backend ??= inTauri ? createTauriBackend() : import("./mock").then((m) => m.createMockBackend());
  return backend;
}

export const isPreview = !inTauri;
