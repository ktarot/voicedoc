import { useState } from "react";
import { LANGUAGES } from "../lib/types";
import type { ModelView, Provider, SettingsView } from "../lib/types";
import { Dialog, Progress } from "./ui";

function KeyField({
  label,
  saved,
  onSave,
}: {
  label: string;
  saved: boolean;
  onSave: (key: string) => Promise<void>;
}) {
  const [value, setValue] = useState("");
  return (
    <div className="field">
      <label>
        {label} {saved && <span className="badge">저장됨</span>}
      </label>
      <div className="row">
        <input
          type="password"
          value={value}
          autoComplete="off"
          placeholder={saved ? "바꾸려면 새 키를 입력하세요" : "API 키를 입력하세요"}
          onChange={(e) => setValue(e.target.value)}
        />
        <button className="btn" disabled={value.trim() === ""} onClick={() => onSave(value).then(() => setValue(""))}>
          저장
        </button>
        {saved && (
          <button className="btn" onClick={() => onSave("")}>
            삭제
          </button>
        )}
      </div>
    </div>
  );
}

export function SettingsDialog({
  settings,
  models,
  downloads,
  onPatch,
  onDownload,
  onCancelDownload,
  onDeleteModel,
  onSetKey,
  onPickFolder,
  onClose,
}: {
  settings: SettingsView;
  models: ModelView[];
  /** 내려받는 중인 모델의 진행률(%) */
  downloads: Record<string, number>;
  onPatch: (patch: Partial<SettingsView>) => void;
  onDownload: (id: string) => void;
  onCancelDownload: (id: string) => void;
  onDeleteModel: (id: string) => void;
  onSetKey: (provider: Provider, key: string) => Promise<void>;
  onPickFolder: () => void;
  onClose: () => void;
}) {
  return (
    <Dialog title="설정" onClose={onClose} wide>
      <section className="settings-section">
        <h3>음성 인식</h3>
        <div className="field">
          <label htmlFor="stt-mode">인식 방식</label>
          <select id="stt-mode" value={settings.sttMode} onChange={(e) => onPatch({ sttMode: e.target.value as SettingsView["sttMode"] })}>
            <option value="local">이 PC에서 인식 (인터넷 불필요)</option>
            <option value="cloud">클라우드 고정밀 인식 (OpenAI, 인터넷·API 키 필요)</option>
          </select>
          {settings.sttMode === "cloud" && (
            <p className="help">음성 파일이 OpenAI 서버로 전송됩니다. 아래에서 OpenAI API 키를 저장해야 합니다.</p>
          )}
        </div>

        <div className="field">
          <label>이 PC용 인식 모델</label>
          <ul className="model-list">
            {models.map((m) => {
              const pct = downloads[m.id];
              const downloading = pct !== undefined;
              return (
                <li key={m.id} className={`model ${settings.modelId === m.id ? "model-selected" : ""}`}>
                  <label className="model-pick">
                    <input
                      type="radio"
                      name="model"
                      checked={settings.modelId === m.id}
                      onChange={() => onPatch({ modelId: m.id })}
                    />
                    <span>
                      <span className="model-name">{m.name}</span>
                      <span className="model-desc">
                        {m.description} · {m.sizeMb >= 1000 ? `${(m.sizeMb / 1000).toFixed(1)}GB` : `${m.sizeMb}MB`}
                      </span>
                    </span>
                  </label>
                  <div className="model-action">
                    {downloading ? (
                      <>
                        <span className="model-pct">{pct}%</span>
                        <button className="btn btn-small" onClick={() => onCancelDownload(m.id)}>
                          중지
                        </button>
                      </>
                    ) : m.downloaded ? (
                      <>
                        <span className="badge">받음</span>
                        <button className="btn btn-small" onClick={() => onDeleteModel(m.id)}>
                          삭제
                        </button>
                      </>
                    ) : (
                      <button className="btn btn-small" onClick={() => onDownload(m.id)}>
                        내려받기
                      </button>
                    )}
                  </div>
                  {downloading && <Progress percent={pct} label={`${m.name} 내려받기`} />}
                </li>
              );
            })}
          </ul>
          <p className="help">모델은 한 번만 내려받으면 되고, 이후에는 인터넷 없이 인식합니다.</p>
        </div>

        <div className="field-grid">
          <div className="field">
            <label htmlFor="language">음성 언어</label>
            <select id="language" value={settings.language} onChange={(e) => onPatch({ language: e.target.value })}>
              {LANGUAGES.map(([code, label]) => (
                <option key={code} value={code}>
                  {label}
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label htmlFor="prompt">인식 힌트 (선택)</label>
            <input
              id="prompt"
              value={settings.prompt}
              placeholder="예: 홍길동 팀장, 분기 실적, VoiceDoc"
              onChange={(e) => onPatch({ prompt: e.target.value })}
            />
          </div>
        </div>
        <p className="help">인식 힌트에 사람 이름이나 전문 용어를 적어 두면 그 단어를 더 정확히 받아 적습니다.</p>
      </section>

      <section className="settings-section">
        <h3>온라인 AI (요약 · 회의록 · 번역)</h3>
        <div className="field">
          <label htmlFor="provider">사용할 AI</label>
          <select id="provider" value={settings.provider} onChange={(e) => onPatch({ provider: e.target.value as Provider })}>
            <option value="claude">Claude (Anthropic)</option>
            <option value="openai">OpenAI</option>
          </select>
        </div>
        <div className="field-grid">
          <KeyField label="Claude API 키" saved={settings.hasClaudeKey} onSave={(k) => onSetKey("claude", k)} />
          <div className="field">
            <label htmlFor="claude-model">Claude 모델</label>
            <input id="claude-model" value={settings.claudeModel} onChange={(e) => onPatch({ claudeModel: e.target.value })} />
          </div>
          <KeyField label="OpenAI API 키" saved={settings.hasOpenaiKey} onSave={(k) => onSetKey("openai", k)} />
          <div className="field">
            <label htmlFor="openai-model">OpenAI 모델</label>
            <input id="openai-model" value={settings.openaiModel} onChange={(e) => onPatch({ openaiModel: e.target.value })} />
          </div>
        </div>
        <div className="field">
          <label htmlFor="cloud-model">클라우드 인식 모델 (OpenAI)</label>
          <input id="cloud-model" value={settings.cloudSttModel} onChange={(e) => onPatch({ cloudSttModel: e.target.value })} />
        </div>
        <p className="help">
          API 키는 이 PC의 보안 저장소에만 보관됩니다. AI 기능을 쓸 때에만 녹취 내용이 선택한 회사의 서버로 전송됩니다.
        </p>
      </section>

      <section className="settings-section">
        <h3>저장</h3>
        <div className="field">
          <label>저장 폴더</label>
          <div className="row">
            <input readOnly value={settings.resolvedOutputDir} />
            <button className="btn" onClick={onPickFolder}>
              바꾸기
            </button>
            {settings.outputDir !== "" && (
              <button className="btn" onClick={() => onPatch({ outputDir: "" })}>
                기본값
              </button>
            )}
          </div>
        </div>
      </section>
    </Dialog>
  );
}
