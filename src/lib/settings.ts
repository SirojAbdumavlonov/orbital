import { useEffect, useState } from "react";
import { create } from "zustand";

export type ThemePref = "system" | "light" | "dark";

/** The one highlight colour (primary buttons, selection, live signal). */
export const ACCENTS = [
  { id: "amber", name: "Amber", dark: "#f2b544", light: "#e3a32c" },
  { id: "blue", name: "Blue", dark: "#6ea8ff", light: "#2f6fe0" },
  { id: "teal", name: "Teal", dark: "#3cc6c0", light: "#1a9690" },
  { id: "green", name: "Green", dark: "#4fc78f", light: "#1e9a62" },
  { id: "violet", name: "Violet", dark: "#a98bff", light: "#7b55e0" },
  { id: "rose", name: "Rose", dark: "#ff7a93", light: "#d9465f" },
  { id: "graphite", name: "Graphite", dark: "#c7cfdd", light: "#4a5670" },
] as const;
export type Accent = (typeof ACCENTS)[number]["id"];

/** When disk and memory (% used) and load (per core) count as a warning or an alarm (Settings → Monitoring). */
export interface Thresholds { diskWarn: number; diskBad: number; memWarn: number; memBad: number; loadWarn: number; loadBad: number }
export const DEFAULT_THRESHOLDS: Thresholds = { diskWarn: 85, diskBad: 95, memWarn: 90, memBad: 95, loadWarn: 1.2, loadBad: 2 };

export interface Settings {
  theme: ThemePref;
  accent: Accent;
  editorFontSize: number;
  editorWordWrap: boolean;
  editorMinimap: boolean;
  terminalFontSize: number;
  /** Seconds between metric samples for the server in view. */
  refreshSecs: number;
  sidebarHidden: boolean;
  /** New terminals open as root when the server's profile can use sudo. */
  terminalAsRoot: boolean;
  /** Look for a new version of Orbital at launch and every few hours. */
  autoUpdates: boolean;
  /** The SSH config "Import from ~/.ssh/config" reads; empty means ~/.ssh/config. */
  sshConfigPath: string;
  /** Suggest commands under the terminal (from the server's shell history and common next steps). */
  terminalSuggestions: boolean;
  /** New terminals run in tmux, so they survive disconnects and can be reattached. */
  terminalTmux: boolean;
  /** At launch, reopen the servers that were open, on the same tab and folder. */
  restoreSession: boolean;
  /** Notify when a connected server needs attention (and when it's fine again). */
  healthAlerts: boolean;
  /** When disk, memory and load count as a warning or an alarm. */
  thresholds: Thresholds;
}

const DEFAULTS: Settings = {
  theme: "system",
  accent: "amber",
  editorFontSize: 13,
  editorWordWrap: false,
  editorMinimap: false,
  terminalFontSize: 13,
  refreshSecs: 2,
  sidebarHidden: false,
  terminalAsRoot: true,
  autoUpdates: true,
  sshConfigPath: "",
  terminalSuggestions: true,
  terminalTmux: false,
  restoreSession: true,
  healthAlerts: true,
  thresholds: DEFAULT_THRESHOLDS,
};

const KEY = "orbital.settings";

function load(): Settings {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "{}");
    return { ...DEFAULTS, ...v, thresholds: { ...DEFAULT_THRESHOLDS, ...(v.thresholds ?? {}) } };
  } catch {
    return DEFAULTS;
  }
}

/** App preferences. They describe this computer, not a server, so they live in local storage. */
export const useSettings = create<Settings & { set(p: Partial<Settings>): void; reset(): void }>((set, get) => ({
  ...load(),
  set(p) {
    set(p);
    const { set: _s, reset: _r, ...values } = { ...get() };
    try {
      localStorage.setItem(KEY, JSON.stringify(values));
    } catch {
      /* private mode: preferences last until the app closes */
    }
  },
  reset() {
    get().set(DEFAULTS);
  },
}));

const dark = () => window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? true;

/** The theme in effect: the user's choice, or the system's when it is "system". */
export function useResolvedTheme(): "light" | "dark" {
  const pref = useSettings((s) => s.theme);
  const [system, setSystem] = useState<"light" | "dark">(dark() ? "dark" : "light");
  useEffect(() => {
    const mq = window.matchMedia?.("(prefers-color-scheme: dark)");
    const on = () => setSystem(mq.matches ? "dark" : "light");
    mq?.addEventListener("change", on);
    return () => mq?.removeEventListener("change", on);
  }, []);
  return pref === "system" ? system : pref;
}

/** Puts the theme and accent on <html data-theme data-accent>, which the stylesheet keys off. */
export function useApplyTheme() {
  const theme = useResolvedTheme();
  const accent = useSettings((s) => s.accent);
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    document.documentElement.dataset.accent = accent;
  }, [theme, accent]);
}

/** The accent colour in effect, for places that need a value rather than a CSS variable (the terminal). */
export function useAccentColor(): string {
  const theme = useResolvedTheme();
  const accent = useSettings((s) => s.accent);
  const a = ACCENTS.find((x) => x.id === accent) ?? ACCENTS[0];
  return theme === "light" ? a.light : a.dark;
}
