import { create } from "zustand";

// The right-hand pane of a tab (the editor in Files, the details panel in
// Services, Docker, Nginx…) can be hidden to give the list the whole width.
// Remembered per tab kind ("files", "services", "tool:docker"), on this computer.

const KEY = "orbital.hiddenPanes";

function load(): string[] {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "[]");
    return Array.isArray(v) ? v.filter((x) => typeof x === "string") : [];
  } catch {
    return [];
  }
}

function save(hidden: string[]) {
  try { localStorage.setItem(KEY, JSON.stringify(hidden)); } catch { /* this run only */ }
}

export const usePanes = create<{ hidden: string[]; toggle(tab: string): void; show(tab: string): void }>((set, get) => ({
  hidden: load(),
  toggle(tab) {
    const h = get().hidden;
    const hidden = h.includes(tab) ? h.filter((x) => x !== tab) : [...h, tab];
    save(hidden);
    set({ hidden });
  },
  show(tab) {
    if (!get().hidden.includes(tab)) return;
    const hidden = get().hidden.filter((x) => x !== tab);
    save(hidden);
    set({ hidden });
  },
}));

/** ⌘\ on macOS, Ctrl+\ elsewhere. */
export const isPaneKey = (e: KeyboardEvent, mac: boolean) =>
  e.key === "\\" && !e.altKey && !e.shiftKey && (mac ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey);
