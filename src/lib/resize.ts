import { useEffect } from "react";

/**
 * Every two-part page can be resized by dragging the line between the parts.
 * The left edge of the second part is the handle (a thin strip drawn by CSS);
 * widths go into CSS variables on <html>, so all pages of a kind share one width,
 * and are remembered. Double-clicking the line restores the default.
 */
const PANELS = [
  // The handle is the `edge` of the element matching `sel`; the width measured
  // from the parent's left or right side goes into CSS variable `v`.
  { sel: ".split > .drawer", edge: "left", v: "--drawer-w", side: "right", min: 260, minOther: 300 },
  { sel: ".files > .editor", edge: "left", v: "--browser-w", side: "left", min: 220, minOther: 320 },
  { sel: ".with-dock.docked > .dock", edge: "left", v: "--dock-w", side: "right", min: 320, minOther: 280 },
  { sel: ".app > .sidebar", edge: "right", v: "--sidebar", side: "left", min: 250, minOther: 640 },
] as const;

const KEY = "orbital.layout";
const EDGE = 6;

function save() {
  const root = document.documentElement.style;
  const v = Object.fromEntries(PANELS.map((p) => [p.v, root.getPropertyValue(p.v)]).filter(([, x]) => x));
  try {
    localStorage.setItem(KEY, JSON.stringify(v));
  } catch { /* not remembered */ }
}

function restore() {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "{}") as Record<string, string>;
    for (const [k, x] of Object.entries(v)) if (PANELS.some((p) => p.v === k)) document.documentElement.style.setProperty(k, x);
  } catch { /* defaults */ }
}

function handleAt(e: MouseEvent) {
  const el = (e.target as Element).closest?.(PANELS.map((p) => p.sel).join(", ")) as HTMLElement | null;
  // Beside a docked editor the side panel sits under the list: nothing to drag there.
  if (!el || el.closest(".docked .dock-main")) return null;
  const p = PANELS.find((x) => el.matches(x.sel))!;
  const r = el.getBoundingClientRect();
  return Math.abs(e.clientX - (p.edge === "left" ? r.left : r.right)) <= EDGE ? { el, p } : null;
}

export function usePanelResizing() {
  useEffect(() => {
    restore();
    const down = (e: PointerEvent) => {
      if (e.button !== 0) return;
      const h = handleAt(e);
      if (!h) return;
      e.preventDefault();
      const box = h.el.parentElement!.getBoundingClientRect();
      document.body.classList.add("resizing");
      const move = (m: PointerEvent) => {
        const raw = h.p.side === "right" ? box.right - m.clientX : m.clientX - box.left;
        const w = Math.max(h.p.min, Math.min(raw, box.width - h.p.minOther));
        document.documentElement.style.setProperty(h.p.v, `${Math.round(w)}px`);
      };
      const up = () => {
        document.body.classList.remove("resizing");
        window.removeEventListener("pointermove", move);
        window.removeEventListener("pointerup", up);
        save();
      };
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", up);
    };
    const dbl = (e: MouseEvent) => {
      const h = handleAt(e);
      if (!h) return;
      document.documentElement.style.removeProperty(h.p.v);
      save();
    };
    window.addEventListener("pointerdown", down, true);
    window.addEventListener("dblclick", dbl, true);
    return () => {
      window.removeEventListener("pointerdown", down, true);
      window.removeEventListener("dblclick", dbl, true);
    };
  }, []);
}
