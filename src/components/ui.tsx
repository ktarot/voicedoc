import type { ReactNode } from "react";

export function Progress({ percent, label }: { percent: number; label?: string }) {
  const value = Math.max(0, Math.min(100, Math.round(percent)));
  return (
    <div className="progress" role="progressbar" aria-valuenow={value} aria-valuemin={0} aria-valuemax={100} aria-label={label}>
      <div className="progress-bar" style={{ width: `${value}%` }} />
    </div>
  );
}

export interface ToastData {
  kind: "ok" | "error";
  message: string;
  action?: { label: string; run: () => void };
}

export function Toast({ toast, onClose }: { toast: ToastData; onClose: () => void }) {
  return (
    <div className={`toast toast-${toast.kind}`} role={toast.kind === "error" ? "alert" : "status"}>
      <span className="toast-message">{toast.message}</span>
      {toast.action && (
        <button className="btn btn-small" onClick={toast.action.run}>
          {toast.action.label}
        </button>
      )}
      <button className="icon-btn" onClick={onClose} aria-label="닫기">
        ✕
      </button>
    </div>
  );
}

export function Dialog({
  title,
  children,
  footer,
  onClose,
  wide,
}: {
  title: string;
  children: ReactNode;
  footer?: ReactNode;
  onClose: () => void;
  wide?: boolean;
}) {
  return (
    <div className="backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className={`dialog ${wide ? "dialog-wide" : ""}`} role="dialog" aria-modal="true" aria-label={title}>
        <header className="dialog-header">
          <h2>{title}</h2>
          <button className="icon-btn" onClick={onClose} aria-label="닫기">
            ✕
          </button>
        </header>
        <div className="dialog-body">{children}</div>
        {footer && <footer className="dialog-footer">{footer}</footer>}
      </div>
    </div>
  );
}
