import { DOC_LABELS, FORMAT_LABELS, TRANSLATE_TARGETS } from "../lib/types";
import type { AiTask, DocKind, Format, Item, SettingsView } from "../lib/types";

const DOC_ORDER: DocKind[] = ["original", "summary", "minutes", "translation"];
const FORMAT_ORDER: Format[] = ["docx", "pdf", "md", "txt"];

export function Workspace({
  item,
  activeDoc,
  settings,
  aiBusy,
  saving,
  onSelectDoc,
  onEdit,
  onToggleTimestamps,
  onRunAi,
  onStopAi,
  onPatchSettings,
  onPickFolder,
  onExport,
}: {
  item: Item;
  activeDoc: DocKind;
  settings: SettingsView;
  /** 지금 AI가 쓰고 있는 문서 종류 */
  aiBusy: DocKind | null;
  saving: boolean;
  onSelectDoc: (kind: DocKind) => void;
  onEdit: (text: string) => void;
  onToggleTimestamps: (on: boolean) => void;
  onRunAi: (task: AiTask) => void;
  onStopAi: () => void;
  onPatchSettings: (patch: Partial<SettingsView>) => void;
  onPickFolder: () => void;
  onExport: () => void;
}) {
  const text = item.docs[activeDoc] ?? "";
  const original = item.docs.original ?? "";
  const tabs = DOC_ORDER.filter((k) => item.docs[k] !== undefined);
  const writing = aiBusy === activeDoc;
  const canAi = original.trim() !== "" && aiBusy === null;
  const hasKey = settings.provider === "claude" ? settings.hasClaudeKey : settings.hasOpenaiKey;

  const toggleFormat = (f: Format) => {
    const formats = settings.formats.includes(f)
      ? settings.formats.filter((x) => x !== f)
      : [...settings.formats, f];
    onPatchSettings({ formats });
  };

  return (
    <section className="workspace">
      <div className="doc-header">
        <h2 className="doc-title" title={item.path}>
          {item.name}
        </h2>
        <div className="tabs" role="tablist">
          {tabs.map((k) => (
            <button
              key={k}
              role="tab"
              aria-selected={k === activeDoc}
              className={`tab ${k === activeDoc ? "tab-active" : ""}`}
              onClick={() => onSelectDoc(k)}
            >
              {DOC_LABELS[k]}
              {aiBusy === k && <span className="dot" aria-label="작성 중" />}
            </button>
          ))}
        </div>
      </div>

      <div className="toolbar">
        <label className="check">
          <input
            type="checkbox"
            checked={settings.timestamps}
            onChange={(e) => onToggleTimestamps(e.target.checked)}
            disabled={!item.transcript}
          />
          시각 표시
        </label>
        <span className="toolbar-sep" />
        <span className="toolbar-label">AI 정리</span>
        <button className="btn btn-small" disabled={!canAi} onClick={() => onRunAi({ kind: "summary" })}>
          요약
        </button>
        <button className="btn btn-small" disabled={!canAi} onClick={() => onRunAi({ kind: "minutes" })}>
          회의록
        </button>
        <span className="translate-group">
          <select
            aria-label="번역할 언어"
            value={settings.translateTarget}
            onChange={(e) => onPatchSettings({ translateTarget: e.target.value })}
          >
            {TRANSLATE_TARGETS.map((t) => (
              <option key={t}>{t}</option>
            ))}
          </select>
          <button
            className="btn btn-small"
            disabled={!canAi}
            onClick={() => onRunAi({ kind: "translate", target: settings.translateTarget })}
          >
            번역
          </button>
        </span>
        {aiBusy && (
          <button className="btn btn-small btn-danger" onClick={onStopAi}>
            AI 중지
          </button>
        )}
        {!hasKey && <span className="hint">AI 기능은 설정에서 API 키를 넣으면 쓸 수 있습니다.</span>}
      </div>

      <textarea
        className="editor"
        value={text}
        readOnly={writing}
        onChange={(e) => onEdit(e.target.value)}
        spellCheck={false}
        aria-label={`${DOC_LABELS[activeDoc]} 내용`}
        placeholder={writing ? "AI가 작성하고 있습니다…" : "내용이 없습니다."}
      />

      <div className="export-bar">
        <div className="formats" role="group" aria-label="저장 형식">
          {FORMAT_ORDER.map((f) => (
            <label key={f} className="check">
              <input type="checkbox" checked={settings.formats.includes(f)} onChange={() => toggleFormat(f)} />
              {FORMAT_LABELS[f]}
            </label>
          ))}
        </div>
        <button className="folder" onClick={onPickFolder} title="저장 폴더 바꾸기">
          <span className="folder-label">저장 폴더</span>
          <span className="folder-path">{"\u200e" + settings.resolvedOutputDir}</span>
        </button>
        <button
          className="btn btn-primary"
          disabled={saving || writing || text.trim() === "" || settings.formats.length === 0}
          onClick={onExport}
        >
          {saving ? "저장 중…" : `${DOC_LABELS[activeDoc]} 저장`}
        </button>
      </div>
    </section>
  );
}
