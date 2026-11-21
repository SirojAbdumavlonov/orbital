import type { TermLaunch } from "./lib/api";
import { toast as sonner } from "sonner";
import { useSettings } from "./lib/settings";
import { create } from "zustand";
import { api, asApiError, ApiError, ConnectOpts, DetectedTool, Metrics, Profile, SessionInfo, Validator , setNotConnectedHandler } from "./lib/api";
import { forgetKept, restoreKept, takeKept } from "./lib/kept";

/** Built-in tabs, plus one `tool:<id>` tab per piece of detected software the user opened. */
/** SFTP sessions are full SSH servers; FTP, WebDAV and S3 offer files only. */
export const isSsh = (s: { info: { protocol?: string } }) => (s.info.protocol ?? "sftp") === "sftp";

export type Tab = "files" | "services" | "monitor" | "terminal" | "activity" | `tool:${string}`;

/** Tabs offered on every server, whatever software it runs. */
export const ALWAYS_TOOLS = ["checkup", "agent", "updates", "cleanup", "deploy", "logs", "tunnels", "ports", "network", "users", "sshkeys"];

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
  /** Runs as root through sudo. */
  root: boolean;
  /** What it runs: a shell (default), a tmux session or a container's shell. */
  launch?: TermLaunch;
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
  /** Software found on the server; null until the first detection finishes. */
  tools: DetectedTool[] | null;
  /** Ids of the tool tabs that are open, in order. Remembered per server. */
  toolTabs: string[];
  /** Domains handed from the nginx tab to the certificates tab's "new certificate" form. */
  certDomains?: string[] | null;
  /** The tool tab showing the editor beside itself (files opened from that tab), if any. */
  editorDock?: Tab | null;
  /** A folder shown beside a tool tab (opened from that tab), instead of switching to Files. */
  folderDock?: { tab: Tab; path: string } | null;
  /** The server's settings changed after connecting; a reconnect applies them. */
  stale?: boolean;
}

/** Where the user is: a server's tab, a global page, or the start page. */
/** A place to go back to: a server's tab (and, in Files, the folder) or a global page. */
export type Loc = { sid: string; tab: Tab; cwd?: string } | { global: NonNullable<State["globalView"]> } | null;

/** Software with its own tab view (the rest get the generic one). */
export const DEDICATED_TOOLS = ["checkup", "agent", "updates", "cleanup", "docker", "nginx", "ufw", "fail2ban", "certbot", "sshd", "ports", "postgresql", "mysql", "mongodb", "redis", "apache", "haproxy", "users", "cron", "deploy", "logs", "tunnels", "network", "sshkeys"];

/** Profile fields that only take effect on a new connection. */
const CONNECTION_FIELDS = ["host", "port", "username", "authMethod", "keyPath", "sudoMode", "initialPath", "jumpProfileId"] as const;
export function connectionChanged(a: Profile, b: Profile): boolean {
  return CONNECTION_FIELDS.some((k) => (a[k] ?? null) !== (b[k] ?? null));
}

export type Prompt =
  // `profileId` is the server being connected to; `opts` is what has been
  // collected so far (secrets typed for earlier hops), carried into the retry.
  | { type: "hostkey"; profileId: string; host: string; port: number; algorithm: string; fingerprint: string; opts?: ConnectOpts }
  | { type: "mismatch"; profileId: string; host: string; port: number; expected: string; actual: string }
  // `forProfileId` is the hop that needs the secret: the server itself or one of its jump hosts.
  | { type: "credentials"; profileId: string; forProfileId: string; what: string; user: string; host: string; error?: string; opts?: ConnectOpts };

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
  editing: Profile | "new" | null;
  /** The "Import from ~/.ssh/config" dialog is open. */
  importingSsh: boolean;
  globalView: "activity" | "knownHosts" | "settings" | "domains" | "uptime" | "snippets" | null;
  /** Back and forward history, like a browser. */
  navBack: Loc[];
  navFwd: Loc[];
  /** Asked after saving changed settings of a connected server. */
  reconnectAsk: string | null;
  /** Server the "New website" dialog is open for. */
  newWebsite: string | null;
  /** Session whose Files tab should open "Send to server" (from ⌘K). */
  sendRequest: string | null;
  /** Bumped when a website was created, so the nginx view reads its config again. */
  websiteRev: number;

  loadProfiles(): Promise<void>;
  goBack(): void;
  goForward(): void;
  /** Close and reopen a server's connection, keeping the tab the user was on. */
  /** A new connection to the same server, keeping the tab, folder, selections and an open New website dialog. Returns the new session id. */
  reconnect(sessionId: string): Promise<string | null>;
  /** After a profile was saved: mark its sessions for reconnect if connection settings changed. */
  profileSaved(before: Profile | null, after: Profile, secretsChanged: boolean): "reconnect" | "none";
  setReconnectAsk(sessionId: string | null): void;
  renameTerminal(sessionId: string, key: string, title: string): void;
  connect(profileId: string, opts?: ConnectOpts): Promise<void>;
  disconnect(sessionId: string): Promise<void>;
  setActive(sessionId: string): void;
  setTab(sessionId: string, tab: Tab): void;
  patch(sessionId: string, p: Partial<SessionState> | ((s: SessionState) => Partial<SessionState>)): void;
  /** Open a shell. `asRoot` defaults to the "terminals as root" setting when this server has sudo. */
  openTerminal(sessionId: string, cwd?: string | null, asRoot?: boolean, launch?: TermLaunch): void;
  closeTerminal(sessionId: string, key: string): void;
  pushMetrics(sessionId: string, m: Metrics): void;
  detectTools(sessionId: string): Promise<void>;
  openTool(sessionId: string, toolId: string): void;
  closeTool(sessionId: string, toolId: string): void;
  /** Open a remote file in the editor: beside a tool tab when opened from one, otherwise in Files. */
  openFile(sessionId: string, path: string): Promise<void>;
  /** Show a remote folder: beside a tool tab when called from one, otherwise in Files. */
  showFolder(sessionId: string, path: string): void;
  setPrompt(p: Prompt | null): void;
  toast(t: Omit<Toast, "id">): void;
  dismiss(id: number): void;
  setEditing(p: Profile | "new" | null): void;
  setImportingSsh(open: boolean): void;
  setGlobalView(v: State["globalView"]): void;
  setNewWebsite(sessionId: string | null): void;
}

const toolTabsKey = (profileId: string) => `orbital.toolTabs.${profileId}`;
function loadToolTabs(profileId: string): string[] | null {
  try {
    const v = JSON.parse(localStorage.getItem(toolTabsKey(profileId)) ?? "null");
    return Array.isArray(v) ? v.filter((x) => typeof x === "string") : null;
  } catch {
    return null;
  }
}
function saveToolTabs(profileId: string, ids: string[]) {
  try {
    localStorage.setItem(toolTabsKey(profileId), JSON.stringify(ids));
  } catch {
    /* private mode or full: tabs just won't be remembered */
  }
}

let termSeq = 0;

export const useStore = create<State>((set, get) => ({
  profiles: [],
  sessions: {},
  order: [],
  active: null,
  connecting: {},
  prompt: null,
  editing: null,
  importingSsh: false,
  globalView: null,
  newWebsite: null,
  sendRequest: null,
  websiteRev: 0,
  navBack: [],
  navFwd: [],
  reconnectAsk: null,

  goBack() {
    const { navBack, navFwd } = get();
    for (let i = navBack.length - 1; i >= 0; i--) {
      if (!reachable(navBack[i])) continue;
      const here = currentLoc(get());
      navigating = true;
      goTo(navBack[i]);
      navigating = false;
      set({ navBack: navBack.slice(0, i), navFwd: [...navFwd, here] });
      return;
    }
  },

  goForward() {
    const { navBack, navFwd } = get();
    for (let i = navFwd.length - 1; i >= 0; i--) {
      if (!reachable(navFwd[i])) continue;
      const here = currentLoc(get());
      navigating = true;
      goTo(navFwd[i]);
      navigating = false;
      set({ navFwd: navFwd.slice(0, i), navBack: [...navBack, here] });
      return;
    }
  },

  async reconnect(sessionId) {
    const s = get().sessions[sessionId];
    if (!s) return null;
    const { tab, cwd, toolTabs } = s;
    set({ reconnectAsk: null });
    // Connect first (the old one counts as lost, so a new one is made), then let the old go:
    // views and dialogs follow the server instead of closing in between.
    get().patch(sessionId, { lost: true });
    await get().connect(s.profile.id);
    const fresh = Object.values(get().sessions).find((x) => x.profile.id === s.profile.id && !x.lost && x.info.id !== sessionId);
    if (!fresh) return null;
    const kept = takeKept(`${sessionId}:`);
    if (get().newWebsite === sessionId) set({ newWebsite: fresh.info.id });
    await get().disconnect(sessionId);
    restoreKept(`${fresh.info.id}:`, kept);
    get().patch(fresh.info.id, { tab: tab === "terminal" ? "files" : tab, cwd, toolTabs });
    set({ active: fresh.info.id, globalView: null });
    return fresh.info.id;
  },

  profileSaved(before, after, secretsChanged) {
    const live = Object.values(get().sessions).filter((x) => x.profile.id === after.id && !x.lost);
    if (live.length === 0) return "none";
    const needs = secretsChanged || (before ? connectionChanged(before, after) : false);
    // Name, colour and group show at once; connection settings wait for a reconnect.
    for (const x of live) get().patch(x.info.id, (cur) => ({ profile: { ...after }, stale: cur.stale || needs }));
    return needs ? "reconnect" : "none";
  },

  setReconnectAsk(reconnectAsk) {
    set({ reconnectAsk });
  },

  renameTerminal(sessionId, key, title) {
    const t = title.trim();
    if (!t) return;
    get().patch(sessionId, (s) => ({ terminals: s.terminals.map((x) => (x.key === key ? { ...x, title: t.slice(0, 40) } : x)) }));
  },

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
    // Clicking the server again while it connects doesn't start a second
    // connection (and a second tab for it): the first one carries on.
    if (get().connecting[profileId] && !opts) return;
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
        tools: null,
        // First visit: Docker opens by itself when the server has it, as before.
        toolTabs: info.protocol !== "sftp" ? [] : loadToolTabs(profileId) ?? (info.dockerVersion ? ["docker"] : []),
      };
      set((s) => ({
        sessions: { ...s.sessions, [info.id]: session },
        order: [...s.order, info.id],
        active: info.id,
        prompt: null,
        globalView: null,
      }));
      get().loadProfiles();
      if (info.protocol === "sftp") get().detectTools(info.id);
    } catch (e) {
      handleConnectError(profileId, asApiError(e), opts);
    } finally {
      set((s) => ({ connecting: { ...s.connecting, [profileId]: false } }));
    }
  },

  async disconnect(sessionId) {
    await api.disconnect(sessionId).catch(() => {});
    forgetKept(`${sessionId}:`);
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

  openTerminal(sessionId, cwd = null, asRoot, launch) {
    const n = ++termSeq;
    const key = `t${n}`;
    const sess = get().sessions[sessionId];
    if (!sess) return;
    const canRoot = sess.info.elevation;
    const root = (asRoot ?? useSettings.getState().terminalAsRoot) && canRoot;
    // With "Open terminals in tmux", plain terminals become tmux sessions
    // orbital-1, orbital-2, …: after a restart the first one picks its session up again.
    const run: TermLaunch = launch ?? (useSettings.getState().terminalTmux ? { kind: "tmux", session: `orbital-${n}` } : { kind: "shell" });
    const where = cwd ? cwd.split("/").filter(Boolean).pop() ?? "/" : null;
    const title =
      run.kind === "tmux" ? `tmux ${run.session}` : run.kind === "container" ? run.id : where ?? (root || sess.profile.username === "root" ? "root" : "shell");
    get().patch(sessionId, (s) => ({
      terminals: [...s.terminals, { key, title, cwd, root: root || s.profile.username === "root", launch: run }],
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

  async detectTools(sessionId) {
    try {
      // Docker's version too: software installed since connecting (from "+") works without reconnecting.
      const [tools, dockerVersion] = await Promise.all([api.detectTools(sessionId), api.dockerVersion(sessionId).catch(() => undefined)]);
      get().patch(sessionId, (s) => {
        if (dockerVersion !== undefined && dockerVersion !== s.info.dockerVersion) s = { ...s, info: { ...s.info, dockerVersion } };
        // Drop remembered tabs for software that is no longer on the server.
        const toolTabs = s.toolTabs.filter((id) => ALWAYS_TOOLS.includes(id) || tools.some((t) => t.id === id) || (id === "docker" && !!s.info.dockerVersion));
        const gone = s.tab.startsWith("tool:") && !toolTabs.includes(s.tab.slice(5));
        return { info: s.info, tools, toolTabs, tab: gone ? "files" : s.tab };
      });
    } catch {
      get().patch(sessionId, (s) => ({ tools: s.tools ?? [] }));
    }
  },

  openTool(sessionId, toolId) {
    get().patch(sessionId, (s) => {
      const toolTabs = s.toolTabs.includes(toolId) ? s.toolTabs : [...s.toolTabs, toolId];
      saveToolTabs(s.profile.id, toolTabs);
      return { toolTabs, tab: `tool:${toolId}` };
    });
  },

  closeTool(sessionId, toolId) {
    get().patch(sessionId, (s) => {
      const toolTabs = s.toolTabs.filter((id) => id !== toolId);
      saveToolTabs(s.profile.id, toolTabs);
      return { toolTabs, tab: s.tab === `tool:${toolId}` ? "files" : s.tab };
    });
  },

  async openFile(sessionId, path) {
    const s = get().sessions[sessionId];
    if (!s) return;
    // From a tool tab (nginx, sshd, …) the editor opens beside it, so the user stays there.
    const show = (cur: SessionState): Partial<SessionState> => (cur.tab.startsWith("tool:") ? { editorDock: cur.tab } : { tab: "files" });
    if (s.files.some((f) => f.path === path)) {
      get().patch(sessionId, (cur) => ({ activeFile: path, ...show(cur) }));
      return;
    }
    try {
      // Checks run on the server, so only SSH servers get them.
      const [fc, validator] = await Promise.all([api.readFile(sessionId, path), isSsh(s) ? api.validatorFor(path) : Promise.resolve(null)]);
      const file: OpenFile = { path: fc.path, content: fc.content, original: fc.content, mtime: fc.mtime, writable: fc.writable, validator };
      get().patch(sessionId, (cur) => ({ files: [...cur.files, file], activeFile: file.path, ...show(cur) }));
    } catch (err) {
      get().toast({ kind: "error", title: `Can't open ${path.split("/").pop()}`, body: asApiError(err).message });
    }
  },

  showFolder(sessionId, path) {
    // From a tool tab the folder opens beside it, so the user stays there.
    get().patch(sessionId, (cur) => (cur.tab.startsWith("tool:") ? { folderDock: { tab: cur.tab, path } } : { cwd: path, tab: "files" }));
  },

  setPrompt(prompt) {
    set({ prompt });
  },

  toast(t) {
    // Notifications are drawn by Sonner (components/Toasts.tsx); errors stay longer.
    const opts = {
      description: t.body,
      duration: t.action ? 12000 : t.kind === "error" ? 9000 : 4500,
      action: t.action ? { label: t.action.label, onClick: t.action.run } : undefined,
    };
    if (t.kind === "success") sonner.success(t.title, opts);
    else if (t.kind === "error") sonner.error(t.title, opts);
    else sonner(t.title, opts);
  },

  dismiss(id) {
    sonner.dismiss(id);
  },

  setImportingSsh(importingSsh) {
    set({ importingSsh });
  },

  setEditing(editing) {
    set({ editing });
  },

  setNewWebsite(newWebsite) {
    set({ newWebsite });
  },

  setGlobalView(globalView) {
    set({ globalView });
  },
}));

// A call that found its connection gone marks the server as disconnected, so its header offers Reconnect.
setNotConnectedHandler((sid) => {
  const s = useStore.getState().sessions[sid];
  if (s && !s.lost) useStore.getState().patch(sid, { lost: true });
});

// ---- back / forward ----

let navigating = false;

export function currentLoc(st: State = useStore.getState()): Loc {
  if (st.globalView) return { global: st.globalView };
  const s = st.active ? st.sessions[st.active] : undefined;
  if (st.active && s) return s.tab === "files" ? { sid: st.active, tab: s.tab, cwd: s.cwd } : { sid: st.active, tab: s.tab };
  return null;
}

export const sameLoc = (a: Loc, b: Loc) => JSON.stringify(a) === JSON.stringify(b);

/** A place can be returned to while its server is still connected. */
export function reachable(l: Loc): boolean {
  if (!l || "global" in l) return true;
  const s = useStore.getState().sessions[l.sid];
  // A closed tool tab is gone too.
  return !!s && (!l.tab.startsWith("tool:") || s.toolTabs.includes(l.tab.slice(5)));
}

/** Go to a place (a server's tab or a global page). */
export function goTo(l: Loc) {
  const st = useStore.getState();
  if (!l) useStore.setState({ globalView: null, active: null });
  else if ("global" in l) useStore.setState({ globalView: l.global });
  else {
    useStore.setState({ globalView: null, active: l.sid });
    st.patch(l.sid, l.cwd ? { tab: l.tab, cwd: l.cwd } : { tab: l.tab });
  }
}

// Every move the user makes (tab, server, global page) becomes a history entry.
useStore.subscribe((st, prev) => {
  if (navigating) return;
  const a = currentLoc(prev);
  const b = currentLoc(st);
  if (sameLoc(a, b)) return;
  useStore.setState({ navBack: [...st.navBack, a].slice(-50), navFwd: [] });
});

function handleConnectError(profileId: string, e: ApiError, opts?: ConnectOpts) {
  const { setPrompt, toast } = useStore.getState();
  // Keep typed secrets for the retry; a fingerprint confirmation is single-use.
  const carried: ConnectOpts = { secret: opts?.secret, hopSecrets: opts?.hopSecrets, remember: opts?.remember };
  switch (e.kind) {
    case "UnknownHostKey":
      setPrompt({ type: "hostkey", profileId, ...e.data, opts: carried });
      break;
    case "HostKeyMismatch":
      setPrompt({ type: "mismatch", profileId, ...e.data });
      break;
    case "CredentialsRequired":
      setPrompt({ type: "credentials", profileId, forProfileId: e.data.profileId ?? profileId, what: e.data.what, user: e.data.user, host: e.data.host, opts: carried });
      break;
    case "AuthFailed": {
      const hopId: string = e.data?.profileId ?? profileId;
      const p = useStore.getState().profiles.find((x) => x.id === hopId);
      if (p && p.authMethod !== "agent") {
        // Drop the rejected secret, keep the ones for other hops.
        const hopSecrets = { ...carried.hopSecrets };
        delete hopSecrets[hopId];
        setPrompt({
          type: "credentials",
          profileId,
          forProfileId: hopId,
          what: p.authMethod === "key" ? "Key passphrase" : "Password",
          user: p.username,
          host: p.host,
          error: "The server rejected these credentials. Try again.",
          opts: { ...carried, hopSecrets, secret: hopId === profileId ? undefined : carried.secret },
        });
      } else {
        toast({ kind: "error", title: "Authentication failed", body: e.message });
      }
      break;
    }
    case "PresenceRequired":
      toast({ kind: "error", title: "Saved credentials are locked", body: e.message });
      break;
    default:
      toast({ kind: "error", title: "Couldn't connect", body: e.message });
  }
}

export const useActiveSession = () => useStore((s) => (s.active ? s.sessions[s.active] : undefined));
