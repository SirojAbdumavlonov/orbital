import { create } from "zustand";
import { useEffect } from "react";
import { Metrics } from "./api";
import { useStore } from "../store";

// The last 24 hours of each server's CPU, memory and fullest disk, one point a
// minute, from the metrics Orbital already reads while it's connected. Kept on
// this computer per server (localStorage `orbital.history.<profile id>`), so the
// Monitor tab and the Home cards can show a day's trend. Gaps are times Orbital
// wasn't connected. Pure parts (`point`, `addPoint`) are tested.

export interface HistoryPoint {
  /** Start of the minute, Unix seconds. */
  t: number;
  cpu: number | null;
  /** % used. */
  mem: number;
  /** % used on the fullest real disk. */
  disk: number | null;
}

export const DAY = 24 * 3600;
const IGNORED_FS = /^(tmpfs|devtmpfs|overlay|squashfs|iso9660|udf|efivarfs|ramfs|nsfs)$/;

export function point(m: Metrics): HistoryPoint {
  const disks = m.disks.filter((d) => !IGNORED_FS.test(d.fsType) && !d.mount.startsWith("/snap/") && d.totalBytes >= 64 * 1024 * 1024);
  return {
    t: Math.floor(m.sampledAtMs / 60_000) * 60,
    cpu: m.cpuPercent === null ? null : Math.round(m.cpuPercent * 10) / 10,
    mem: m.memTotal ? Math.round((1 - m.memAvailable / m.memTotal) * 1000) / 10 : 0,
    disk: disks.length ? Math.round(Math.max(...disks.map((d) => d.usedPercent)) * 10) / 10 : null,
  };
}

/** Adds a point (replacing one for the same minute) and drops what's older than a day. */
export function addPoint(points: HistoryPoint[], p: HistoryPoint): HistoryPoint[] {
  const kept = points.filter((x) => x.t > p.t - DAY && x.t !== p.t);
  kept.push(p);
  kept.sort((a, b) => a.t - b.t);
  return kept;
}

const key = (profileId: string) => `orbital.history.${profileId}`;

function load(profileId: string): HistoryPoint[] {
  try {
    const v = JSON.parse(localStorage.getItem(key(profileId)) ?? "[]");
    const now = Date.now() / 1000;
    return Array.isArray(v) ? v.filter((p) => p && typeof p.t === "number" && p.t > now - DAY) : [];
  } catch {
    return [];
  }
}

export const useHistory = create<{ byProfile: Record<string, HistoryPoint[]> }>(() => ({ byProfile: {} }));

export function historyOf(profileId: string): HistoryPoint[] {
  const have = useHistory.getState().byProfile[profileId];
  if (have) return have;
  const loaded = load(profileId);
  useHistory.setState((s) => ({ byProfile: { ...s.byProfile, [profileId]: loaded } }));
  return loaded;
}

/** Mounted once in App: records each connected server's latest sample once a minute, and saves. */
export function useHistoryRecorder() {
  useEffect(() => {
    let lastSeen: Record<string, number> = {};
    const unsub = useStore.subscribe((st) => {
      for (const s of Object.values(st.sessions)) {
        const m = s.metrics[s.metrics.length - 1];
        if (!m || s.lost) continue;
        if (lastSeen[s.info.id] === m.sampledAtMs) continue;
        lastSeen[s.info.id] = m.sampledAtMs;
        const pid = s.profile.id;
        const next = addPoint(historyOf(pid), point(m));
        useHistory.setState((h) => ({ byProfile: { ...h.byProfile, [pid]: next } }));
      }
    });
    // Saved once a minute (and when Orbital closes), not on every sample.
    const save = () => {
      for (const [pid, pts] of Object.entries(useHistory.getState().byProfile)) {
        try { localStorage.setItem(key(pid), JSON.stringify(pts)); } catch { /* this run only */ }
      }
    };
    const t = setInterval(save, 60_000);
    window.addEventListener("beforeunload", save);
    return () => {
      unsub();
      clearInterval(t);
      window.removeEventListener("beforeunload", save);
      save();
      lastSeen = {};
    };
  }, []);
}
