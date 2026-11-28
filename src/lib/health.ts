import { create } from "zustand";
import { useEffect } from "react";
import { api, Metrics } from "./api";
import { goTo, isSsh, useStore } from "../store";
import { DEFAULT_THRESHOLDS, Thresholds, useSettings } from "./settings";
import { isMainWindow } from "./windows";

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
// fails, and when an alerted server has been fine for three checks in a row.
// A server that keeps going back and forth (a value sitting on a threshold)
// needs five red checks to alert again within 30 minutes of recovering, so
// it can't flood the phone. Per-server mute and a global switch (Settings →
// Monitoring).

export interface AlertState {
  /** Checks in a row the server has been red. */
  redFor: number;
  /** Checks in a row it has been fine since an alert. */
  okFor: number;
  /** A "needs attention" alert was sent and not yet cleared. */
  alerted: boolean;
  /** Failed services already told about. */
  failed: string[];
  /** When it last came back to normal (ms), for the flapping brake. */
  recoveredAt: number | null;
}

export const NO_ALERTS: AlertState = { redFor: 0, okFor: 0, alerted: false, failed: [], recoveredAt: null };

/** Checks in a row before "back to normal". */
export const CALM_CHECKS = 3;
const FLAP_WINDOW_MS = 30 * 60_000;

export interface AlertMessage {
  kind: "bad" | "ok" | "warn";
  title: string;
  body: string;
}

/** One check: the next state and what to tell the user. Pure; tested. */
export function alertStep(prev: AlertState, server: string, h: Health, failed: string[], now = Date.now()): { next: AlertState; messages: AlertMessage[] } {
  const messages: AlertMessage[] = [];
  if (h.level === "unknown") return { next: prev, messages };
  const newlyFailed = failed.filter((u) => !prev.failed.includes(u));
  const redFor = h.level === "bad" ? prev.redFor + 1 : 0;
  const fine = h.level !== "bad" && failed.length === 0;
  let okFor = fine ? prev.okFor + 1 : 0;
  let alerted = prev.alerted;
  let recoveredAt = prev.recoveredAt;
  // Just back from an alert: wait longer before alerting again.
  const need = recoveredAt !== null && now - recoveredAt < FLAP_WINDOW_MS ? 5 : 2;
  if (newlyFailed.length) {
    // Services read "nginx failed"; containers come described ("container api is unhealthy").
    const what = newlyFailed.map((u) => (u.startsWith("container ") ? u.slice("container ".length) : `${u.replace(/\.service$/, "")} failed`));
    const names = newlyFailed.map((u) => (u.startsWith("container ") ? u.slice("container ".length).split(" ")[0] : u.replace(/\.service$/, "")));
    messages.push({ kind: "bad", title: `${server}: ${what.length === 1 ? what[0] : `${what.length} problems`}`, body: what.length === 1 ? names[0] : what.join(", ") });
    alerted = true;
    okFor = 0;
  } else if (redFor === need && !prev.alerted) {
    messages.push({ kind: "bad", title: `${server} needs attention`, body: h.reasons.slice(0, 3).join(" · ") });
    alerted = true;
  }
  if (prev.alerted && fine && okFor >= CALM_CHECKS) {
    messages.push({ kind: "ok", title: `${server} is back to normal`, body: h.level === "warn" ? h.reasons.slice(0, 2).join(" · ") : "Nothing needs attention." });
    alerted = false;
    okFor = 0;
    recoveredAt = now;
  }
  return { next: { redFor, okFor, alerted, failed, recoveredAt }, messages };
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
/** Each server's last uptime reading and when it was taken, to notice a restart. */
const lastUptime: Record<string, { uptime: number; at: number }> = {};

/**
 * The server restarted between two readings: its uptime is shorter than the
 * time since the last one (it started counting again in between). An older
 * reading turning up later (from before the Mac slept, or a connection that
 * was just replaced) is not a restart. Pure; tested.
 */
export function restartMessage(server: string, prev: { uptime: number; at: number } | undefined, uptime: number, at: number): AlertMessage | null {
  if (!prev || at <= prev.at) return null;
  const elapsed = (at - prev.at) / 1000;
  if (uptime > elapsed + 120 || uptime + 300 > prev.uptime + elapsed) return null;
  const mins = Math.max(1, Math.round(uptime / 60));
  return { kind: "warn", title: `${server} restarted`, body: `Up again for ${mins < 120 ? `${mins} min` : `${Math.round(mins / 60)} h`}; it had been up ${Math.round(prev.uptime / 3600)} h. If you didn't restart it, check its logs.` };
}

/** Metrics older than this don't count for alerts (a connection that stopped updating). */
export const FRESH_MS = 3 * 60_000;

/**
 * The connection to judge each server by: one per server (the one with the
 * newest numbers), only open, shown ones, and only when its numbers are fresh.
 * Pure; tested.
 */
export function sessionsToCheck<T extends { info: { id: string }; profile: { id: string }; lost?: boolean; metrics: { sampledAtMs: number }[] }>(sessions: Record<string, T>, order: string[], now: number): T[] {
  const best = new Map<string, T>();
  for (const id of order) {
    const s = sessions[id];
    const m = s?.metrics[s.metrics.length - 1];
    if (!s || s.lost || !m || now - m.sampledAtMs > FRESH_MS) continue;
    const cur = best.get(s.profile.id);
    if (!cur || cur.metrics[cur.metrics.length - 1].sampledAtMs < m.sampledAtMs) best.set(s.profile.id, s);
  }
  return [...best.values()];
}

/** "Show" for an alert: the server's connection as it is now (it may have been replaced since), else Home. */
function showServer(profileId: string) {
  const st = useStore.getState();
  const id = st.order.find((x) => st.sessions[x]?.profile.id === profileId && !st.sessions[x].lost);
  useStore.setState({ globalView: null });
  if (id) st.setActive(id);
  else goTo(null);
}

/** Runs after each minute's check: notifies (and toasts) health changes worth knowing. */
export function checkAlerts() {
  // Alerts go out once: from the main window, not from every window showing the server.
  if (!isMainWindow || !useSettings.getState().healthAlerts) return;
  const st = useStore.getState();
  const muted = useHealthMute.getState().muted;
  const now = Date.now();
  for (const s of sessionsToCheck(st.sessions, st.order, now)) {
    if (!isSsh(s)) continue;
    const last = s.metrics[s.metrics.length - 1];
    const failed = useFailed.getState().bySession[s.info.id] ?? [];
    const boxes = useFailed.getState().containers[s.info.id] ?? [];
    const h = assess(last, failed, boxes, useSettings.getState().thresholds);
    // Keyed by server, so a reconnect doesn't alert again.
    // Containers in trouble alert like failed services: at once, and once.
    const { next, messages } = alertStep(alertStates[s.profile.id] ?? NO_ALERTS, s.profile.name, h, [...failed, ...boxes.map((b) => `container ${b}`)], now);
    alertStates[s.profile.id] = next;
    if (typeof last.uptimeSecs === "number") {
      const restart = restartMessage(s.profile.name, lastUptime[s.profile.id], last.uptimeSecs, last.sampledAtMs);
      if (!lastUptime[s.profile.id] || last.sampledAtMs > lastUptime[s.profile.id].at) lastUptime[s.profile.id] = { uptime: last.uptimeSecs, at: last.sampledAtMs };
      if (restart) messages.push(restart);
    }
    if (muted.includes(s.profile.id)) continue;
    const profileId = s.profile.id;
    for (const m of messages) {
      // Also to Telegram, email or the AI agent, when set up (Settings → Notifications).
      void api.sendAlert({ kind: "health", level: m.kind, title: m.title, body: m.body }).catch(() => {});
      st.toast({ kind: m.kind === "bad" ? "error" : m.kind === "warn" ? "info" : "success", title: m.title, body: m.body, action: { label: "Show", run: () => showServer(profileId) } });
      if (document.visibilityState !== "visible" || !document.hasFocus()) void api.notify(m.title, m.body).catch(() => {});
    }
  }
}
