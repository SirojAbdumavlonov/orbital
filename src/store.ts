import { create } from "zustand";
import { api, asApiError, ApiError, Metrics, Profile, SessionInfo, Validator } from "./lib/api";

export type Tab = "files" | "services" | "docker" | "monitor" | "terminal" | "activity";

export interface OpenFile {
  path: string;
  content: string;
  original: string;
  mtime: number | null;
  writable: boolean;
  validator: Validator | null;
}

export interface TermTab {
  key: string;
  title: string;
  cwd: string | null;
}

export interface SessionState {
  info: SessionInfo;
  profile: Profile;
  tab: Tab;
  cwd: string;
  files: OpenFile[];
  activeFile: string | null;
  terminals: TermTab[];
  activeTerminal: string | null;
  metrics: Metrics[]; // rolling history, newest last
  lost: boolean;
}

export type Prompt =
  | { type: "hostkey"; profileId: string; host: string; port: number; algorithm: string; fingerprint: string; secret?: string; remember?: boolean }
  | { type: "mismatch"; profileId: string; host: string; port: number; expected: string; actual: string }
  | { type: "credentials"; profileId: string; what: string; user: string; host: string; error?: string };

export interface Toast {
  id: number;
  kind: "info" | "success" | "error";
  title: string;
  body?: string;
  action?: { label: string; run: () => void };
}

interface State {
  profiles: Profile[];
  sessions: Record<string, SessionState>;
  order: string[];
  active: string | null;
  connecting: Record<string, boolean>;
  prompt: Prompt | null;
  toasts: Toast[];
  editing: Profile | "new" | null;
  globalView: "activity" | "knownHosts" | null;

  loadProfiles(): Promise<void>;
  connect(profileId: string, opts?: { acceptFingerprint?: string; secret?: string; remember?: boolean }): Promise<void>;
  disconnect(sessionId: string): Promise<void>;
  setActive(sessionId: string): void;
  setTab(sessionId: string, tab: Tab): void;
  patch(sessionId: string, p: Partial<SessionState> | ((s: SessionState) => Partial<SessionState>)): void;
  openTerminal(sessionId: string, cwd?: string | null): void;
  closeTerminal(sessionId: string, key: string): void;
  pushMetrics(sessionId: string, m: Metrics): void;
  setPrompt(p: Prompt | null): void;
  toast(t: Omit<Toast, "id">): void;
  dismiss(id: number): void;
  setEditing(p: Profile | "new" | null): void;
  setGlobalView(v: State["globalView"]): void;
}

let toastId = 0;
let termSeq = 0;

export const useStore = create<State>((set, get) => ({
  profiles: [],
  sessions: {},
  order: [],
  active: null,
  connecting: {},
  prompt: null,
  toasts: [],
  editing: null,
  globalView: null,

  async loadProfiles() {
    set({ profiles: await api.listProfiles() });
  },

  async connect(profileId, opts) {
    const existing = Object.values(get().sessions).find((s) => s.profile.id === profileId && !s.lost);
    if (existing && !opts) {
      set({ active: existing.info.id, globalView: null });
      return;
    }
    const profile = get().profiles.find((p) => p.id === profileId);
    if (!profile) return;
    set((s) => ({ connecting: { ...s.connecting, [profileId]: true } }));
    try {
      const info = await api.connect(profileId, opts);
      const session: SessionState = {
        info,
        profile,
        tab: "files",
        cwd: info.home,
        files: [],
        activeFile: null,
        terminals: [],
        activeTerminal: null,
        metrics: [],
        lost: false,
      };
      set((s) => ({
        sessions: { ...s.sessions, [info.id]: session },
        order: [...s.order, info.id],
        active: info.id,
        prompt: null,
        globalView: null,
      }));
      get().loadProfiles();
    } catch (e) {
      handleConnectError(profileId, asApiError(e), opts);
    } finally {
      set((s) => ({ connecting: { ...s.connecting, [profileId]: false } }));
    }
  },

  async disconnect(sessionId) {
    await api.disconnect(sessionId).catch(() => {});
    set((s) => {
      const sessions = { ...s.sessions };
      delete sessions[sessionId];
      const order = s.order.filter((id) => id !== sessionId);
      return { sessions, order, active: s.active === sessionId ? order[order.length - 1] ?? null : s.active };
    });
  },

  setActive(sessionId) {
    set({ active: sessionId, globalView: null });
  },

  setTab(sessionId, tab) {
    get().patch(sessionId, { tab });
  },

  patch(sessionId, p) {
    set((s) => {
      const cur = s.sessions[sessionId];
      if (!cur) return {};
      const delta = typeof p === "function" ? p(cur) : p;
      return { sessions: { ...s.sessions, [sessionId]: { ...cur, ...delta } } };
    });
  },

  openTerminal(sessionId, cwd = null) {
    const key = `t${++termSeq}`;
    const title = cwd ? cwd.split("/").filter(Boolean).pop() ?? "/" : "shell";
    get().patch(sessionId, (s) => ({
      terminals: [...s.terminals, { key, title, cwd }],
      activeTerminal: key,
      tab: "terminal",
    }));
  },

  closeTerminal(sessionId, key) {
    get().patch(sessionId, (s) => {
      const terminals = s.terminals.filter((t) => t.key !== key);
      return { terminals, activeTerminal: s.activeTerminal === key ? terminals[terminals.length - 1]?.key ?? null : s.activeTerminal };
    });
  },

  pushMetrics(sessionId, m) {
    get().patch(sessionId, (s) => ({ metrics: [...s.metrics.slice(-89), m] }));
  },

  setPrompt(prompt) {
    set({ prompt });
  },

  toast(t) {
    const id = ++toastId;
    set((s) => ({ toasts: [...s.toasts, { ...t, id }] }));
    setTimeout(() => get().dismiss(id), t.action ? 12000 : t.kind === "error" ? 9000 : 4500);
  },

  dismiss(id) {
    set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }));
  },

  setEditing(editing) {
    set({ editing });
  },

  setGlobalView(globalView) {
    set({ globalView });
  },
}));

function handleConnectError(profileId: string, e: ApiError, opts?: { secret?: string; remember?: boolean }) {
  const { setPrompt, toast } = useStore.getState();
  switch (e.kind) {
    case "UnknownHostKey":
      setPrompt({ type: "hostkey", profileId, ...e.data, secret: opts?.secret, remember: opts?.remember });
      break;
    case "HostKeyMismatch":
      setPrompt({ type: "mismatch", profileId, ...e.data });
      break;
    case "CredentialsRequired":
      setPrompt({ type: "credentials", profileId, ...e.data });
      break;
    case "AuthFailed": {
      const p = useStore.getState().profiles.find((x) => x.id === profileId);
      if (p && p.authMethod !== "agent") {
        setPrompt({
          type: "credentials",
          profileId,
          what: p.authMethod === "key" ? "Key passphrase" : "Password",
          user: p.username,
          host: p.host,
          error: "The server rejected these credentials. Try again.",
        });
      } else {
        toast({ kind: "error", title: "Authentication failed", body: e.message });
      }
      break;
    }
    default:
      toast({ kind: "error", title: "Couldn't connect", body: e.message });
  }
}

export const useActiveSession = () => useStore((s) => (s.active ? s.sessions[s.active] : undefined));
