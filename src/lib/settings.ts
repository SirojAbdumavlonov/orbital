import { useEffect, useState } from "react";
import { create } from "zustand";

export type ThemePref = "system" | "light" | "dark";

export interface Settings {
  theme: ThemePref;
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
}

const DEFAULTS: Settings = {
  theme: "system",
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
};

const KEY = "orbital.settings";

function load(): Settings {
  try {
    return { ...DEFAULTS, ...JSON.parse(localStorage.getItem(KEY) ?? "{}") };
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

/** Puts the theme on <html data-theme>, which the stylesheet keys off. */
export function useApplyTheme() {
  const theme = useResolvedTheme();
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);
}
