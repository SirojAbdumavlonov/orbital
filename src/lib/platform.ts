// macOS or Windows (or Linux): shortcuts, wording and window chrome differ.

const ua = typeof navigator === "undefined" ? "" : `${navigator.platform} ${navigator.userAgent}`;
export const isMac = /Mac/i.test(ua);
export const isWindows = /Win/i.test(ua);

/** The command key is down: ⌘ on macOS, Ctrl elsewhere. */
export const mod = (e: { metaKey: boolean; ctrlKey: boolean }) => (isMac ? e.metaKey : e.ctrlKey);

/** Shortcut labels as each platform writes them. */
export const KEYS = isMac
  ? { commandMenu: "⌘K", back: "⌘[", forward: "⌘]", newServer: "⌘N", sidebar: "⌃⌘S", delete: "⌘⌫", copy: "⌘C", paste: "⌘V", pane: "⌘\\", shortcuts: "⌘/", gotoFolder: "⇧⌘G", zoomIn: "⌘+", zoomOut: "⌘−", zoomReset: "⌘0" }
  : { commandMenu: "Ctrl+K", back: "Alt+←", forward: "Alt+→", newServer: "Ctrl+N", sidebar: "Ctrl+Shift+S", delete: "Delete", copy: "Ctrl+C", paste: "Ctrl+V", pane: "Ctrl+\\", shortcuts: "Ctrl+/", gotoFolder: "Ctrl+Shift+G", zoomIn: "Ctrl++", zoomOut: "Ctrl+−", zoomReset: "Ctrl+0" };

/** Is this the shortcut that shows and hides the sidebar? */
export const isSidebarKey = (e: KeyboardEvent) =>
  e.key.toLowerCase() === "s" && (isMac ? e.metaKey && e.ctrlKey : e.ctrlKey && e.shiftKey && !e.altKey);

/** Back (-1) or forward (1) for ⌘[ ⌘] on macOS and Alt+← Alt+→ elsewhere; 0 for anything else. */
export function navKey(e: KeyboardEvent): -1 | 0 | 1 {
  if (isMac) return e.metaKey && e.key === "[" ? -1 : e.metaKey && e.key === "]" ? 1 : 0;
  if (!e.altKey || e.ctrlKey || e.metaKey) return 0;
  return e.key === "ArrowLeft" ? -1 : e.key === "ArrowRight" ? 1 : 0;
}

/** Running as the desktop app (Tauri), not in a plain browser (`npm run demo`). */
export const isApp = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window && !import.meta.env.VITE_DEMO;

/**
 * Set once at startup so the stylesheet can follow the platform (window buttons, title bar).
 * In a browser there are no window buttons to leave room for, whatever the computer.
 */
export function markPlatform() {
  document.documentElement.dataset.platform = !isApp ? "web" : isMac ? "mac" : isWindows ? "windows" : "linux";
}
