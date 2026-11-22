import { create } from "zustand";
import { NetCheck, NetResult } from "./api";

// Network check results per server (profile), kept outside the view: a
// traceroute still running when you switch tabs lands here and shows when you
// come back, and the last 30 results (with their output) are remembered on
// this computer between launches.

export interface NetRun {
  n: number;
  check: NetCheck;
  target: string;
  port: number | null;
  /** Unix milliseconds. */
  at: number;
  result?: NetResult;
  error?: string;
  /** Still running (only in this run of Orbital; a check interrupted by quitting is marked as such). */
  busy?: boolean;
}

export const MAX_RUNS = 30;
const MAX_OUTPUT = 20_000;
const key = (profileId: string) => `orbital.netruns.${profileId}`;

/** What's stored: finished runs only, output trimmed; a run left busy by quitting says so. */
export function forStorage(runs: NetRun[]): NetRun[] {
  return runs.slice(0, MAX_RUNS).map((r) => {
    const out: NetRun = { ...r, busy: undefined };
    if (r.busy) out.error = "Orbital closed before this check finished";
    if (out.result && out.result.output.length > MAX_OUTPUT) out.result = { ...out.result, output: `${out.result.output.slice(0, MAX_OUTPUT)}\n… (cut)` };
    return out;
  });
}

function load(profileId: string): NetRun[] {
  try {
    const v = JSON.parse(localStorage.getItem(key(profileId)) ?? "[]");
    return Array.isArray(v) ? v.filter((r) => r && typeof r.n === "number" && typeof r.target === "string") : [];
  } catch {
    return [];
  }
}

export const useNetRuns = create<{
  byProfile: Record<string, NetRun[]>;
  list(profileId: string): NetRun[];
  add(profileId: string, r: NetRun): void;
  update(profileId: string, n: number, p: Partial<NetRun>): void;
  clear(profileId: string): void;
}>((set, get) => {
  const put = (profileId: string, runs: NetRun[]) => {
    set((s) => ({ byProfile: { ...s.byProfile, [profileId]: runs } }));
    try { localStorage.setItem(key(profileId), JSON.stringify(forStorage(runs))); } catch { /* this run only */ }
  };
  return {
    byProfile: {},
    list(profileId) {
      const have = get().byProfile[profileId];
      if (have) return have;
      const loaded = load(profileId);
      set((s) => ({ byProfile: { ...s.byProfile, [profileId]: loaded } }));
      return loaded;
    },
    add(profileId, r) {
      put(profileId, [r, ...get().list(profileId)].slice(0, MAX_RUNS));
    },
    update(profileId, n, p) {
      put(profileId, get().list(profileId).map((r) => (r.n === n ? { ...r, ...p } : r)));
    },
    clear(profileId) {
      put(profileId, []);
    },
  };
});
