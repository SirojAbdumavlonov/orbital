import { X } from "lucide-react";
import NumberFlow from "@number-flow/react";
import * as Dialog from "@radix-ui/react-dialog";
import { ReactNode, useEffect, useState } from "react";
import { create } from "zustand";

/**
 * A dialog, built on Radix Dialog: focus stays inside while it is open, Esc and
 * a click outside close it (when `onClose` is given), and screen readers get
 * its title and description.
 */
export function Modal(props: { title: string; subtitle?: ReactNode; children: ReactNode; footer?: ReactNode; wide?: boolean; onClose?: () => void }) {
  return (
    <Dialog.Root open onOpenChange={(open) => !open && props.onClose?.()}>
      <Dialog.Portal>
        <Dialog.Overlay className="scrim">
          <Dialog.Content
            className={`modal ${props.wide ? "wide" : ""}`}
            {...(props.subtitle ? {} : { "aria-describedby": undefined })}
            onEscapeKeyDown={(e) => !props.onClose && e.preventDefault()}
            onPointerDownOutside={(e) => !props.onClose && e.preventDefault()}
          >
            <div className="modal-head">
              <Dialog.Title asChild><h2>{props.title}</h2></Dialog.Title>
              {props.subtitle && <Dialog.Description asChild><p>{props.subtitle}</p></Dialog.Description>}
            </div>
            <div className="modal-body">{props.children}</div>
            {props.footer && <div className="modal-foot">{props.footer}</div>}
          </Dialog.Content>
        </Dialog.Overlay>
      </Dialog.Portal>
    </Dialog.Root>
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
      {req.body && <div className="muted" style={{ whiteSpace: "pre-line" }}>{req.body}</div>}
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

/** Closes a detail panel; Esc does the same unless a dialog is open or the user is typing. */
export function DrawerClose({ onClose }: { onClose: () => void }) {
  useEffect(() => {
    const h = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || document.querySelector(".modal")) return;
      if ((e.target as Element).closest?.("input, textarea, select, .monaco-editor, .xterm")) return;
      onClose();
    };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, [onClose]);
  return (
    <button className="icon-btn drawer-close" aria-label="Close details" title="Close (Esc)" onClick={onClose}>
      <X size={14} />
    </button>
  );
}

/** A live number that rolls to its new value (NumberFlow). Shows "—" while unknown. */
export function LiveNumber({ value, decimals = 0, suffix }: { value: number | null | undefined; decimals?: number; suffix?: string }) {
  if (value == null || !Number.isFinite(value)) return <>—</>;
  return <NumberFlow value={value} format={{ minimumFractionDigits: decimals, maximumFractionDigits: decimals }} suffix={suffix} willChange />;
}
