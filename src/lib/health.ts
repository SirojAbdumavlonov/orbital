import { create } from "zustand";
import { useEffect } from "react";
import { api, Metrics } from "./api";
import { isSsh, useStore } from "../store";
import { DEFAULT_THRESHOLDS, Thresholds, useSettings } from "./settings";

// How a connected server is doing, at a glance (the sidebar's dot and the
// overview on Home): from its latest metrics (already polled) and the failed
// services (polled here every minute). Pure rules in `assess`, tested.

export type HealthLevel = "ok" | "warn" | "bad" | "unknown";
export interface Health {
  level: HealthLevel;
  /** Short reasons, worst first ("Disk / 96%", "2 services failed"). */
  reasons: string[];
}

/** File systems that are always "full" or don't matter for space. */
const IGNORED_FS = /^(tmpfs|devtmpfs|overlay|squashfs|iso9660|udf|efivarfs|ramfs|nsfs|fuse\..*)$/;

/** Containers in trouble: stuck restarting, or reported unhealthy by their own health check. */
export function troubledContainers(list: { names: string; state: string; status: string }[]): string[] {
  return list
    .filter((c) => c.state === "restarting" || /\(unhealthy\)/.test(c.status))
    .map((c) => `${c.names} ${c.state === "restarting" ? "keeps restarting" : "is unhealthy"}`);
}


export function assess(m: Metrics | undefined, failed: string[] | undefined, containers: string[] = [], th: Thresholds = DEFAULT_THRESHOLDS): Health {
  if (!m) return { level: "unknown", reasons: [] };
  const bad: string[] = [];
  const warn: string[] = [];
  for (const d of m.disks) {
    if (IGNORED_FS.test(d.fsType) || d.mount.startsWith("/snap/") || d.totalBytes < 64 * 1024 * 1024) continue;
    const pct = Math.round(d.usedPercent);
    if (pct >= th.diskBad) bad.push(`Disk ${d.mount} ${pct}%`);
    else if (pct >= th.diskWarn) warn.push(`Disk ${d.mount} ${pct}%`);
  }
  if (m.memTotal > 0) {
    const used = 100 - (m.memAvailable / m.memTotal) * 100;
    if (used >= th.memBad) bad.push(`Memory ${Math.round(used)}% used`);
    else if (used >= th.memWarn) warn.push(`Memory ${Math.round(used)}% used`);
  }
  if (m.cores > 0) {
    const perCore = m.load[0] / m.cores;
    if (perCore >= th.loadBad) bad.push(`Load ${m.load[0].toFixed(1)} on ${m.cores} cores`);
    else if (perCore >= th.loadWarn) warn.push(`Load ${m.load[0].toFixed(1)} on ${m.cores} cores`);
  }
  if (failed && failed.length) {
    const names = failed.map((u) => u.replace(/\.service$/, ""));
    warn.push(failed.length === 1 ? `${names[0]} failed` : `${failed.length} services failed (${names.slice(0, 3).join(", ")}${failed.length > 3 ? ", …" : ""})`);
  }
  for (const c of containers.slice(0, 3)) warn.push(`Container ${c}`);
  if (containers.length > 3) warn.push(`${containers.length - 3} more containers in trouble`);
  const reasons = [...bad, ...warn];
  return { level: bad.length ? "bad" : warn.length ? "warn" : "ok", reasons };
}

/** Failed services per connected server (session id), and containers in trouble. */
export const useFailed = create<{ bySession: Record<string, string[]>; containers: Record<string, string[]> }>(() => ({ bySession: {}, containers: {} }));

const FAILED_EVERY_MS = 60_000;

/** Mounted once in App: checks failed services on every connected SSH server with systemd, every minute. */
export function useFailedPolling() {
  useEffect(() => {
    let stop = false;
    const tick = async () => {
      const sessions = Object.values(useStore.getState().sessions).filter((s) => !s.lost && isSsh(s));
      const next: Record<string, string[]> = {};
      const boxes: Record<string, string[]> = {};
      await Promise.all(
        sessions.map(async (s) => {
          const prev = useFailed.getState();
          // Services (systemd) and containers (Docker): wherever the server runs its software.
          if (s.info.hasSystemd) {
            try {
              next[s.info.id] = await api.failedUnits(s.info.id);
            } catch {
              // Unknown this time: keep what we had.
              if (prev.bySession[s.info.id]) next[s.info.id] = prev.bySession[s.info.id];
            }
          }
          if (s.info.dockerVersion || s.tools?.some((t) => t.id === "docker")) {
            try {
              boxes[s.info.id] = troubledContainers(await api.listContainers(s.info.id));
            } catch {
              if (prev.containers[s.info.id]) boxes[s.info.id] = prev.containers[s.info.id];
            }
          }
        }),
      );
      if (!stop) {
        useFailed.setState({ bySession: next, containers: boxes });
        checkAlerts();
      }
    };
    void tick();
    const t = setInterval(tick, FAILED_EVERY_MS);
    // A newly connected server is checked soon, not a minute later.
    let count = Object.keys(useStore.getState().sessions).length;
    const unsub = useStore.subscribe((st) => {
      const n = Object.keys(st.sessions).length;
      if (n > count) setTimeout(tick, 3000);
      count = n;
    });
    return () => {
      stop = true;
      clearInterval(t);
      unsub();
    };
  }, []);
}

/** The health of a connected session: latest metrics plus failed services. */
export function useSessionHealth(sid: string | undefined): Health {
  const m = useStore((st) => (sid ? st.sessions[sid]?.metrics[st.sessions[sid].metrics.length - 1] : undefined));
  const failed = useFailed((f) => (sid ? f.bySession[sid] : undefined));
  const boxes = useFailed((f) => (sid ? f.containers[sid] : undefined));
  const th = useSettings((x) => x.thresholds);
  return assess(m, failed, boxes, th);
}

// ---------- alerts ----------
//
// A notification when a server turns red and stays red for two checks in a
// row (a minute apart, so a load spike doesn't alert), at once when a service
// fails, and when an alerted server is back to normal. Per-server mute and a
// global switch (Settings → Monitoring).

export interface AlertState {
  /** Checks in a row the server has been red. */
  redFor: number;
  /** A "needs attention" alert was sent and not yet cleared. */
  alerted: boolean;
  /** Failed services already told about. */
  failed: string[];
}

export const NO_ALERTS: AlertState = { redFor: 0, alerted: false, failed: [] };

export interface AlertMessage {
  kind: "bad" | "ok" | "warn";
  title: string;
  body: string;
}

/** One check: the next state and what to tell the user. Pure; tested. */
export function alertStep(prev: AlertState, server: string, h: Health, failed: string[]): { next: AlertState; messages: AlertMessage[] } {
  const messages: AlertMessage[] = [];
  if (h.level === "unknown") return { next: prev, messages };
  const newlyFailed = failed.filter((u) => !prev.failed.includes(u));
  const redFor = h.level === "bad" ? prev.redFor + 1 : 0;
  let alerted = prev.alerted;
  if (newlyFailed.length) {
    // Services read "nginx failed"; containers come described ("container api is unhealthy").
    const what = newlyFailed.map((u) => (u.startsWith("container ") ? u.slice("container ".length) : `${u.replace(/\.service$/, "")} failed`));
    const names = newlyFailed.map((u) => (u.startsWith("container ") ? u.slice("container ".length).split(" ")[0] : u.replace(/\.service$/, "")));
    messages.push({ kind: "bad", title: `${server}: ${what.length === 1 ? what[0] : `${what.length} problems`}`, body: what.length === 1 ? names[0] : what.join(", ") });
    alerted = true;
  } else if (redFor === 2 && !prev.alerted) {
    messages.push({ kind: "bad", title: `${server} needs attention`, body: h.reasons.slice(0, 3).join(" · ") });
    alerted = true;
  }
  if (prev.alerted && h.level !== "bad" && failed.length === 0 && !newlyFailed.length) {
    messages.push({ kind: "ok", title: `${server} is back to normal`, body: h.level === "warn" ? h.reasons.slice(0, 2).join(" · ") : "Nothing needs attention." });
    alerted = false;
  }
  return { next: { redFor, alerted, failed }, messages };
}

/** Servers whose alerts are muted (profile ids), on this computer. */
const MUTE_KEY = "orbital.healthMuted";
export const useHealthMute = create<{ muted: string[]; toggle(profileId: string): void }>((set, get) => ({
  muted: (() => { try { return JSON.parse(localStorage.getItem(MUTE_KEY) ?? "[]"); } catch { return []; } })(),
  toggle(profileId) {
    const m = get().muted;
    const muted = m.includes(profileId) ? m.filter((x) => x !== profileId) : [...m, profileId];
    try { localStorage.setItem(MUTE_KEY, JSON.stringify(muted)); } catch { /* this run only */ }
    set({ muted });
  },
}));

const alertStates: Record<string, AlertState> = {};
/** Each server's uptime at the last check, to notice a restart. */
const lastUptime: Record<string, number> = {};

/** The server restarted since the last check: its uptime went down. Pure; tested. */
export function restartMessage(server: string, prevUptime: number | undefined, uptime: number): AlertMessage | null {
  if (prevUptime === undefined || uptime + 120 >= prevUptime) return null;
  const mins = Math.max(1, Math.round(uptime / 60));
  return { kind: "warn", title: `${server} restarted`, body: `Up again for ${mins < 120 ? `${mins} min` : `${Math.round(mins / 60)} h`}; it had been up ${Math.round(prevUptime / 3600)} h. If you didn't restart it, check its logs.` };
}

/** Runs after each minute's check: notifies (and toasts) health changes worth knowing. */
export function checkAlerts() {
  if (!useSettings.getState().healthAlerts) return;
  const st = useStore.getState();
  const muted = useHealthMute.getState().muted;
  for (const s of Object.values(st.sessions)) {
    if (s.lost || !isSsh(s)) continue;
    const failed = useFailed.getState().bySession[s.info.id] ?? [];
    const boxes = useFailed.getState().containers[s.info.id] ?? [];
    const h = assess(s.metrics[s.metrics.length - 1], failed, boxes, useSettings.getState().thresholds);
    // Keyed by server, so a reconnect doesn't alert again.
    // Containers in trouble alert like failed services: at once, and once.
    const { next, messages } = alertStep(alertStates[s.profile.id] ?? NO_ALERTS, s.profile.name, h, [...failed, ...boxes.map((b) => `container ${b}`)]);
    alertStates[s.profile.id] = next;
    const up = s.metrics[s.metrics.length - 1]?.uptimeSecs;
    if (typeof up === "number") {
      const restart = restartMessage(s.profile.name, lastUptime[s.profile.id], up);
      lastUptime[s.profile.id] = up;
      if (restart) messages.push(restart);
    }
    if (muted.includes(s.profile.id)) continue;
    for (const m of messages) {
      // Also to Telegram, email or the AI agent, when set up (Settings → Notifications).
      void api.sendAlert({ kind: "health", level: m.kind, title: m.title, body: m.body }).catch(() => {});
      st.toast({ kind: m.kind === "bad" ? "error" : m.kind === "warn" ? "info" : "success", title: m.title, body: m.body, action: { label: "Show", run: () => { useStore.setState({ globalView: null }); st.setActive(s.info.id); } } });
      if (document.visibilityState !== "visible" || !document.hasFocus()) void api.notify(m.title, m.body).catch(() => {});
    }
  }
}
