import { useCallback, useEffect, useRef, useState } from "react";
import { FileList } from "./components/FileList";
import { SettingsDialog } from "./components/SettingsDialog";
import { Dialog, Progress, Toast } from "./components/ui";
import type { ToastData } from "./components/ui";
import { Workspace } from "./components/Workspace";
import { api, isPreview } from "./lib/api";
import { DOC_LABELS } from "./lib/types";
import type { AiTask, DocKind, Item, ModelView, Provider, Settings, SettingsView } from "./lib/types";

const AI_DOC: Record<AiTask["kind"], DocKind> = {
  summary: "summary",
  minutes: "minutes",
  translate: "translation",
};

let nextId = 1;
const newId = () => `job-${Date.now()}-${nextId++}`;

function fileName(path: string): string {
  return path.split(/[\\/]/).pop() || path;
}

function stem(name: string): string {
  const i = name.lastIndexOf(".");
  return i > 0 ? name.slice(0, i) : name;
}

const isCancelled = (e: unknown) => String(e).includes("취소");

export default function App() {
  const [settings, setSettings] = useState<SettingsView | null>(null);
  const [models, setModels] = useState<ModelView[]>([]);
  const [downloads, setDownloads] = useState<Record<string, number>>({});
  const [items, setItemsState] = useState<Item[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [activeDoc, setActiveDoc] = useState<DocKind>("original");
  const [running, setRunning] = useState(false);
  const [aiJob, setAiJob] = useState<{ jobId: string; itemId: string; doc: DocKind } | null>(null);
  const [saving, setSaving] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [confirmTimestamps, setConfirmTimestamps] = useState<boolean | null>(null);
  const [toast, setToast] = useState<ToastData | null>(null);
  const [dropHover, setDropHover] = useState(false);

  // 대기열 처리 루프가 항상 최신 목록을 보도록 ref와 상태를 함께 갱신한다.
  const itemsRef = useRef<Item[]>([]);
  const runningRef = useRef(false);
  const aiJobRef = useRef(aiJob);
  aiJobRef.current = aiJob;

  const setItems = useCallback((fn: (items: Item[]) => Item[]) => {
    itemsRef.current = fn(itemsRef.current);
    setItemsState(itemsRef.current);
  }, []);

  const updateItem = useCallback(
    (id: string, patch: Partial<Item> | ((item: Item) => Partial<Item>)) => {
      setItems((list) =>
        list.map((it) => (it.id === id ? { ...it, ...(typeof patch === "function" ? patch(it) : patch) } : it)),
      );
    },
    [setItems],
  );

  const fail = useCallback((e: unknown) => setToast({ kind: "error", message: String(e) }), []);

  const reloadSettings = useCallback(async () => {
    const b = await api();
    setSettings(await b.getSettings());
  }, []);

  const reloadModels = useCallback(async () => {
    const b = await api();
    setModels(await b.listModels());
  }, []);

  const addFiles = useCallback(
    (paths: string[]) => {
      const known = new Set(itemsRef.current.map((i) => i.path));
      const fresh = paths
        .filter((p) => !known.has(p))
        .map<Item>((path) => ({
          id: newId(),
          path,
          name: fileName(path),
          status: "queued",
          percent: 0,
          edited: false,
          docs: {},
        }));
      if (fresh.length === 0) return;
      setItems((list) => [...list, ...fresh]);
      setSelectedId((cur) => cur ?? fresh[0].id);
    },
    [setItems],
  );

  // 처음 한 번: 설정·모델을 읽고 백엔드 이벤트를 듣는다.
  useEffect(() => {
    let disposed = false;
    const unlisten: (() => void)[] = [];
    (async () => {
      const b = await api();
      await Promise.all([reloadSettings(), reloadModels()]);
      const subs = await Promise.all([
        b.onJobProgress((p) =>
          updateItem(p.jobId, (it) =>
            it.status === "decoding" || it.status === "transcribing"
              ? { status: p.stage === "decode" ? "decoding" : "transcribing", percent: p.percent }
              : {},
          ),
        ),
        b.onModelProgress((p) =>
          setDownloads((d) => ({ ...d, [p.id]: p.total ? Math.floor((p.done * 100) / p.total) : 0 })),
        ),
        b.onAiDelta((d) => {
          const job = aiJobRef.current;
          if (job?.jobId !== d.jobId) return;
          updateItem(job.itemId, (it) => ({ docs: { ...it.docs, [job.doc]: (it.docs[job.doc] ?? "") + d.delta } }));
        }),
        b.onFileDrop(addFiles, setDropHover),
      ]);
      if (disposed) subs.forEach((u) => u());
      else unlisten.push(...subs);
    })().catch(fail);
    return () => {
      disposed = true;
      unlisten.forEach((u) => u());
    };
  }, [addFiles, fail, reloadModels, reloadSettings, updateItem]);

  useEffect(() => {
    if (toast?.kind !== "ok") return;
    const t = setTimeout(() => setToast(null), 8000);
    return () => clearTimeout(t);
  }, [toast]);

  const patchSettings = useCallback(
    async (patch: Partial<SettingsView>) => {
      if (!settings) return;
      const next = { ...settings, ...patch };
      setSettings(next);
      const { hasClaudeKey, hasOpenaiKey, resolvedOutputDir, audioExtensions, ...plain } = next;
      void hasClaudeKey, hasOpenaiKey, resolvedOutputDir, audioExtensions;
      try {
        const b = await api();
        await b.saveSettings(plain as Settings);
        if ("outputDir" in patch) await reloadSettings();
      } catch (e) {
        fail(e);
      }
    },
    [settings, reloadSettings, fail],
  );

  const pickFiles = async () => {
    try {
      const b = await api();
      addFiles(await b.pickFiles(settings?.audioExtensions ?? []));
    } catch (e) {
      fail(e);
    }
  };

  const pickFolder = async () => {
    try {
      const b = await api();
      const dir = await b.pickFolder();
      if (dir) await patchSettings({ outputDir: dir });
    } catch (e) {
      fail(e);
    }
  };

  const runQueue = async () => {
    if (runningRef.current) return;
    runningRef.current = true;
    setRunning(true);
    try {
      const b = await api();
      for (;;) {
        const next = itemsRef.current.find((i) => i.status === "queued");
        if (!next) break;
        updateItem(next.id, { status: "decoding", percent: 0, error: undefined });
        try {
          const r = await b.transcribe(next.id, next.path);
          updateItem(next.id, {
            status: "done",
            percent: 100,
            transcript: r.transcript,
            edited: false,
            docs: { original: r.text },
          });
        } catch (e) {
          updateItem(next.id, isCancelled(e) ? { status: "cancelled" } : { status: "error", error: String(e) });
        }
      }
    } finally {
      runningRef.current = false;
      setRunning(false);
    }
  };

  const cancelItem = async (id: string) => (await api()).cancelJob(id).catch(fail);

  const removeItem = (id: string) => {
    setItems((list) => list.filter((i) => i.id !== id));
    if (selectedId === id) {
      setSelectedId(itemsRef.current[0]?.id ?? null);
      setActiveDoc("original");
    }
  };

  const downloadModel = async (id: string) => {
    setDownloads((d) => ({ ...d, [id]: 0 }));
    try {
      const b = await api();
      await b.downloadModel(id);
      setToast({ kind: "ok", message: "인식 모델을 내려받았습니다." });
    } catch (e) {
      if (!isCancelled(e)) fail(e);
    } finally {
      setDownloads(({ [id]: _, ...rest }) => rest);
      await reloadModels().catch(fail);
    }
  };

  const selected = items.find((i) => i.id === selectedId) ?? null;

  const applyTimestamps = async (on: boolean) => {
    setConfirmTimestamps(null);
    await patchSettings({ timestamps: on });
    if (!selected?.transcript) return;
    try {
      const b = await api();
      const text = await b.formatTranscript(selected.transcript, on);
      updateItem(selected.id, (it) => ({ edited: false, docs: { ...it.docs, original: text } }));
    } catch (e) {
      fail(e);
    }
  };

  const runAi = async (task: AiTask) => {
    if (!selected || aiJobRef.current) return;
    const doc = AI_DOC[task.kind];
    const job = { jobId: newId(), itemId: selected.id, doc };
    const previous = selected.docs[doc];
    aiJobRef.current = job;
    setAiJob(job);
    updateItem(selected.id, (it) => ({ docs: { ...it.docs, [doc]: "" } }));
    setActiveDoc(doc);
    try {
      const b = await api();
      const out = await b.runAi(job.jobId, task, selected.docs.original ?? "");
      updateItem(job.itemId, (it) => ({ docs: { ...it.docs, [doc]: out.text } }));
      if (out.truncated) {
        setToast({ kind: "error", message: "AI 출력이 길이 제한에 걸려 끝부분이 잘렸습니다." });
      }
    } catch (e) {
      // 실패하면 이전 결과로 되돌리고, 처음 만든 문서였다면 탭을 없앤다.
      updateItem(job.itemId, (it) => {
        const docs = { ...it.docs };
        if (previous === undefined) delete docs[doc];
        else docs[doc] = previous;
        return { docs };
      });
      if (previous === undefined) setActiveDoc("original");
      if (!isCancelled(e)) fail(e);
    } finally {
      aiJobRef.current = null;
      setAiJob(null);
    }
  };

  const exportDoc = async () => {
    if (!selected || !settings) return;
    const body = selected.docs[activeDoc] ?? "";
    const base = stem(selected.name);
    const title = activeDoc === "original" ? base : `${base} - ${DOC_LABELS[activeDoc]}`;
    setSaving(true);
    try {
      const b = await api();
      const paths = await b.exportDocument(title, body, settings.formats);
      setToast({
        kind: "ok",
        message: paths.length === 1 ? `저장했습니다: ${fileName(paths[0])}` : `파일 ${paths.length}개를 저장했습니다.`,
        action: { label: "폴더 열기", run: () => b.reveal(paths[0]).catch(fail) },
      });
    } catch (e) {
      fail(e);
    } finally {
      setSaving(false);
    }
  };

  if (!settings) {
    return (
      <div className="loading">
        {toast?.kind === "error" ? (
          <div className="placeholder">
            <h2>앱을 시작하지 못했습니다</h2>
            <p className="error-text">{toast.message}</p>
            <button className="btn btn-primary" onClick={() => window.location.reload()}>
              다시 시도
            </button>
          </div>
        ) : (
          "불러오는 중…"
        )}
      </div>
    );
  }

  const queued = items.filter((i) => i.status === "queued").length;
  const model = models.find((m) => m.id === settings.modelId);
  const needsModel = settings.sttMode === "local" && model !== undefined && !model.downloaded;
  const needsKey = settings.sttMode === "cloud" && !settings.hasOpenaiKey;
  const modelPct = model ? downloads[model.id] : undefined;

  return (
    <div className={`app ${dropHover ? "app-drop" : ""}`}>
      <header className="topbar">
        <h1>
          VoiceDoc <span className="tagline">음성을 문서로</span>
        </h1>
        {isPreview && <span className="badge badge-warn">미리보기 (가짜 데이터)</span>}
        <div className="topbar-actions">
          <button className="btn btn-primary" onClick={pickFiles}>
            파일 추가
          </button>
          <button className="btn" onClick={() => setShowSettings(true)}>
            설정
          </button>
        </div>
      </header>

      <div className="layout">
        <aside className="sidebar">
          <div className="sidebar-head">
            <h2>파일</h2>
            <span className="mode">
              {settings.sttMode === "local" ? `이 PC에서 인식 · ${model?.name ?? settings.modelId}` : "클라우드 인식"}
            </span>
          </div>

          {needsModel && model && (
            <div className="notice">
              {modelPct === undefined ? (
                <>
                  <p>
                    처음 한 번 인식 모델({model.sizeMb}MB)을 내려받아야 합니다. 이후에는 인터넷 없이 동작합니다.
                  </p>
                  <button className="btn btn-primary" onClick={() => downloadModel(model.id)}>
                    모델 내려받기
                  </button>
                </>
              ) : (
                <>
                  <p>모델 내려받는 중… {modelPct}%</p>
                  <Progress percent={modelPct} label="모델 내려받기" />
                  <button className="btn btn-small" onClick={() => cancelItem(`model:${model.id}`)}>
                    중지
                  </button>
                </>
              )}
            </div>
          )}
          {needsKey && (
            <div className="notice">
              <p>클라우드 인식을 쓰려면 설정에서 OpenAI API 키를 저장해 주세요.</p>
              <button className="btn" onClick={() => setShowSettings(true)}>
                설정 열기
              </button>
            </div>
          )}

          <FileList
            items={items}
            selectedId={selectedId}
            onSelect={(id) => {
              setSelectedId(id);
              setActiveDoc("original");
            }}
            onRemove={removeItem}
            onCancel={cancelItem}
            onRetry={(id) => updateItem(id, { status: "queued", percent: 0, error: undefined })}
          />

          <button
            className="btn btn-primary btn-block"
            disabled={queued === 0 || running || needsModel || needsKey}
            onClick={runQueue}
          >
            {running ? "변환 중…" : queued > 0 ? `변환 시작 (${queued}개)` : "변환 시작"}
          </button>
        </aside>

        <main className="main">
          {selected?.status === "done" ? (
            <Workspace
              item={selected}
              activeDoc={selected.docs[activeDoc] !== undefined ? activeDoc : "original"}
              settings={settings}
              aiBusy={aiJob?.itemId === selected.id ? aiJob.doc : null}
              saving={saving}
              onSelectDoc={setActiveDoc}
              onEdit={(text) =>
                updateItem(selected.id, (it) => ({
                  edited: it.edited || activeDoc === "original",
                  docs: { ...it.docs, [activeDoc]: text },
                }))
              }
              onToggleTimestamps={(on) => (selected.edited ? setConfirmTimestamps(on) : applyTimestamps(on))}
              onRunAi={runAi}
              onStopAi={() => aiJob && cancelItem(aiJob.jobId)}
              onPatchSettings={patchSettings}
              onPickFolder={pickFolder}
              onExport={exportDoc}
            />
          ) : (
            <div className="placeholder">
              {selected ? (
                <>
                  <h2>{selected.name}</h2>
                  {selected.status === "error" ? (
                    <p className="error-text">{selected.error}</p>
                  ) : selected.status === "queued" ? (
                    <p>「변환 시작」을 누르면 이 파일을 글로 바꿉니다.</p>
                  ) : selected.status === "cancelled" ? (
                    <p>변환을 중지했습니다. 목록에서 「다시」를 누르면 다시 시도합니다.</p>
                  ) : (
                    <>
                      <p>{selected.status === "decoding" ? "오디오를 읽고 있습니다…" : "음성을 글로 바꾸고 있습니다…"}</p>
                      <Progress percent={selected.percent} label="변환 진행률" />
                    </>
                  )}
                </>
              ) : (
                <>
                  <div className="drop-art" aria-hidden="true">
                    🎙️ → 📄
                  </div>
                  <h2>음성 파일을 여기에 끌어다 놓으세요</h2>
                  <p>m4a, mp3, mp4, wav 등을 글로 바꿔 워드·PDF·마크다운·텍스트로 저장합니다.</p>
                  <button className="btn btn-primary" onClick={pickFiles}>
                    파일 선택
                  </button>
                </>
              )}
            </div>
          )}
        </main>
      </div>

      {showSettings && (
        <SettingsDialog
          settings={settings}
          models={models}
          downloads={downloads}
          onPatch={patchSettings}
          onDownload={downloadModel}
          onCancelDownload={(id) => cancelItem(`model:${id}`)}
          onDeleteModel={async (id) => {
            try {
              await (await api()).deleteModel(id);
              await reloadModels();
            } catch (e) {
              fail(e);
            }
          }}
          onSetKey={async (provider: Provider, key: string) => {
            try {
              await (await api()).setApiKey(provider, key);
              await reloadSettings();
            } catch (e) {
              fail(e);
            }
          }}
          onPickFolder={pickFolder}
          onClose={() => setShowSettings(false)}
        />
      )}

      {confirmTimestamps !== null && (
        <Dialog
          title="원문을 다시 만들까요?"
          onClose={() => setConfirmTimestamps(null)}
          footer={
            <>
              <button className="btn" onClick={() => setConfirmTimestamps(null)}>
                그대로 두기
              </button>
              <button className="btn btn-danger" onClick={() => applyTimestamps(confirmTimestamps)}>
                다시 만들기
              </button>
            </>
          }
        >
          <p>시각 표시를 바꾸면 원문을 인식 결과에서 다시 만듭니다. 원문에서 직접 고친 내용은 사라집니다.</p>
        </Dialog>
      )}

      {toast && <Toast toast={toast} onClose={() => setToast(null)} />}
    </div>
  );
}
