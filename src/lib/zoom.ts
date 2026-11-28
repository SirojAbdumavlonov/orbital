import { useEffect } from "react";
import { isMac } from "./platform";
import { useSettings } from "./settings";

// Zoom with the usual shortcuts, ⌘+ ⌘− ⌘0 (Ctrl on Windows and Linux): in a
// terminal they change its font size, in the editor the editor's, anywhere
// else the size of the whole interface (the webview's own zoom, so the
// terminal and editor stay crisp and their mouse positions right). Every size
// is a setting, kept on this computer. Pure parts are tested.

export type ZoomKey = "in" | "out" | "reset";
export const UI_ZOOM = { min: 0.7, max: 1.6, step: 0.1 };
export const FONT = { min: 8, max: 32 };

/** ⌘= ⌘+ ⌘− ⌘0 (and the keypad's), Ctrl elsewhere; null for anything else. */
export function zoomKey(e: Pick<KeyboardEvent, "key" | "code" | "metaKey" | "ctrlKey" | "altKey">, mac = isMac): ZoomKey | null {
  if ((mac ? !e.metaKey : !e.ctrlKey) || e.altKey) return null;
  if (e.key === "=" || e.key === "+" || e.code === "NumpadAdd" || e.code === "Equal") return "in";
  if (e.key === "-" || e.key === "_" || e.code === "NumpadSubtract" || e.code === "Minus") return "out";
  if (e.key === "0" || e.code === "Digit0" || e.code === "Numpad0") return "reset";
  return null;
}

export function nextUiZoom(z: number, k: ZoomKey): number {
  if (k === "reset") return 1;
  const v = Math.round((z + (k === "in" ? UI_ZOOM.step : -UI_ZOOM.step)) * 10) / 10;
  return Math.min(UI_ZOOM.max, Math.max(UI_ZOOM.min, v));
}

export function nextFont(size: number, k: ZoomKey, base = 13): number {
  if (k === "reset") return base;
  return Math.min(FONT.max, Math.max(FONT.min, size + (k === "in" ? 1 : -1)));
}

/** Applies the interface zoom: the webview's own zoom in the app, CSS zoom in a plain browser (the demo). */
export async function applyUiZoom(z: number) {
  try {
    if ("__TAURI_INTERNALS__" in window && !import.meta.env.VITE_DEMO) {
      const { getCurrentWebview } = await import("@tauri-apps/api/webview");
      await getCurrentWebview().setZoom(z);
      return;
    }
  } catch { /* fall back to CSS */ }
  (document.documentElement.style as CSSStyleDeclaration & { zoom: string }).zoom = z === 1 ? "" : String(z);
}

/** Mounted once in App: applies the saved interface zoom and handles the shortcuts. */
export function useZoom() {
  const ui = useSettings((s) => s.uiZoom);
  useEffect(() => { void applyUiZoom(ui); }, [ui]);
  useEffect(() => {
    const on = (e: KeyboardEvent) => {
      const k = zoomKey(e);
      if (!k) return;
      e.preventDefault();
      e.stopPropagation();
      const st = useSettings.getState();
      const t = e.target as Element | null;
      const where = typeof t?.closest === "function" ? (t.closest(".xterm") ? "terminal" : t.closest(".monaco-editor") ? "editor" : "ui") : "ui";
      let what: string;
      if (where === "terminal") {
        st.set({ terminalFontSize: nextFont(st.terminalFontSize, k) });
        what = `Terminal text ${useSettings.getState().terminalFontSize} pt`;
      } else if (where === "editor") {
        st.set({ editorFontSize: nextFont(st.editorFontSize, k) });
        what = `Editor text ${useSettings.getState().editorFontSize} pt`;
      } else {
        st.set({ uiZoom: nextUiZoom(st.uiZoom, k) });
        what = `Interface ${Math.round(useSettings.getState().uiZoom * 100)}%`;
      }
      showZoom(what);
    };
    // Capture: before the terminal or the editor sees the keys.
    window.addEventListener("keydown", on, true);
    return () => window.removeEventListener("keydown", on, true);
  }, []);
}

/** One small note that updates while the keys are pressed (the same toast id), not a pile of toasts. */
function showZoom(text: string) {
  void import("sonner").then(({ toast }) => toast(text, { id: "orbital-zoom", duration: 1200 }));
}
