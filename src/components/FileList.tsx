import type { Item, ItemStatus } from "../lib/types";
import { Progress } from "./ui";

const STATUS: Record<ItemStatus, string> = {
  queued: "대기 중",
  decoding: "오디오 읽는 중",
  transcribing: "인식 중",
  done: "완료",
  error: "실패",
  cancelled: "취소됨",
};

export function FileList({
  items,
  selectedId,
  onSelect,
  onRemove,
  onCancel,
  onRetry,
}: {
  items: Item[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  onRemove: (id: string) => void;
  onCancel: (id: string) => void;
  onRetry: (id: string) => void;
}) {
  if (items.length === 0) {
    return <p className="empty-note">아직 추가한 파일이 없습니다.</p>;
  }
  return (
    <ul className="file-list">
      {items.map((item) => {
        const busy = item.status === "decoding" || item.status === "transcribing";
        return (
          <li key={item.id} className={`file ${item.id === selectedId ? "file-selected" : ""}`}>
            <button className="file-main" onClick={() => onSelect(item.id)} title={item.path}>
              <span className="file-name">{item.name}</span>
              <span className={`status status-${item.status}`}>
                {STATUS[item.status]}
                {busy && ` ${Math.round(item.percent)}%`}
              </span>
              {busy && <Progress percent={item.percent} label={`${item.name} 진행률`} />}
              {item.status === "error" && <span className="file-error">{item.error}</span>}
            </button>
            <div className="file-actions">
              {busy ? (
                <button className="btn btn-small" onClick={() => onCancel(item.id)}>
                  중지
                </button>
              ) : (
                <>
                  {(item.status === "error" || item.status === "cancelled") && (
                    <button className="btn btn-small" onClick={() => onRetry(item.id)}>
                      다시
                    </button>
                  )}
                  <button className="icon-btn" onClick={() => onRemove(item.id)} aria-label={`${item.name} 목록에서 빼기`}>
                    ✕
                  </button>
                </>
              )}
            </div>
          </li>
        );
      })}
    </ul>
  );
}
