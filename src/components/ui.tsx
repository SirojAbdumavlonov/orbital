import { ReactNode, useEffect, useState } from "react";
import { create } from "zustand";

export function Modal(props: { title: string; subtitle?: ReactNode; children: ReactNode; footer?: ReactNode; wide?: boolean; onClose?: () => void }) {
  useEffect(() => {
    const h = (e: KeyboardEvent) => e.key === "Escape" && props.onClose?.();
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, [props.onClose]);
  return (
    <div className="scrim" onMouseDown={(e) => e.target === e.currentTarget && props.onClose?.()}>
      <div className={`modal ${props.wide ? "wide" : ""}`} role="dialog" aria-modal="true" aria-label={props.title}>
        <div className="modal-head">
          <h2>{props.title}</h2>
          {props.subtitle && <p>{props.subtitle}</p>}
        </div>
        <div className="modal-body">{props.children}</div>
        {props.footer && <div className="modal-foot">{props.footer}</div>}
      </div>
    </div>
  );
}

// ---- promise-based confirmation dialog ----
interface ConfirmReq {
  title: string;
  body?: ReactNode;
  confirmLabel: string;
  danger?: boolean;
  /** User must type this to enable the button (for irreversible actions). */
  requireText?: string;
  resolve: (ok: boolean) => void;
}
const useConfirmStore = create<{ req: ConfirmReq | null; set: (r: ConfirmReq | null) => void }>((set) => ({
  req: null,
  set: (req) => set({ req }),
}));

export function confirm(req: Omit<ConfirmReq, "resolve">): Promise<boolean> {
  return new Promise((resolve) => useConfirmStore.getState().set({ ...req, resolve }));
}

export function ConfirmHost() {
  const { req, set } = useConfirmStore();
  const [typed, setTyped] = useState("");
  useEffect(() => setTyped(""), [req]);
  if (!req) return null;
  const done = (ok: boolean) => {
    req.resolve(ok);
    set(null);
  };
  const blocked = !!req.requireText && typed !== req.requireText;
  return (
    <Modal
      title={req.title}
      onClose={() => done(false)}
      footer={
        <>
          <button className="btn" onClick={() => done(false)}>Cancel</button>
          <button className={`btn ${req.danger ? "danger" : "primary"}`} disabled={blocked} autoFocus={!req.requireText} onClick={() => done(true)}>
            {req.confirmLabel}
          </button>
        </>
      }
    >
      {req.body && <div className="muted">{req.body}</div>}
      {req.requireText && (
        <label className="field">
          <span>Type <b className="mono">{req.requireText}</b> to confirm</span>
          <input className="input mono" autoFocus value={typed} onChange={(e) => setTyped(e.target.value)} onKeyDown={(e) => e.key === "Enter" && !blocked && done(true)} />
        </label>
      )}
    </Modal>
  );
}

export function Sparkline({ values, max = 100, width = 96, height = 22, fluid }: { values: number[]; max?: number; width?: number; height?: number; fluid?: boolean }) {
  const size = fluid ? { width: "100%", height, viewBox: `0 0 ${width} ${height}`, preserveAspectRatio: "none" } : { width, height };
  if (values.length < 2) return <svg {...size} aria-hidden />;
  const m = Math.max(max, ...values, 1);
  const step = width / (values.length - 1);
  const pts = values.map((v, i) => `${(i * step).toFixed(1)},${(height - 2 - (v / m) * (height - 4)).toFixed(1)}`);
  return (
    <svg {...size} aria-hidden>
      <polygon points={`0,${height} ${pts.join(" ")} ${width},${height}`} fill="var(--amber-soft)" />
      <polyline points={pts.join(" ")} fill="none" stroke="var(--amber)" strokeWidth="1.5" strokeLinejoin="round" vectorEffect="non-scaling-stroke" />
    </svg>
  );
}

export function StateDot({ state, label, nonzero }: { state: string; label?: string; nonzero?: boolean }) {
  return (
    <span className={`state ${state} ${nonzero ? "nonzero" : ""}`}>
      <i />
      {label ?? state}
    </span>
  );
}

export function Bar({ value, cache, hot }: { value: number; cache?: number; hot?: boolean }) {
  return (
    <div className={`bar ${hot ? "hot" : ""}`}>
      <i style={{ width: `${Math.min(100, Math.max(0, value))}%` }} />
      {cache != null && <i className="cache" style={{ width: `${Math.min(100, Math.max(0, cache))}%` }} />}
    </div>
  );
}
