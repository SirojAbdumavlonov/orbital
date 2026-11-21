import { useEffect } from "react";
import { create } from "zustand";
import { api, StatsState } from "./api";

/** What anonymous usage statistics send; shown in the question and in Settings. */
export const STATS_WHAT =
  "When Orbital opens and every 30 minutes while it's open: the app version, your OS, and two numbers, how many servers are connected and how many are saved. A random install id, tied to nothing, counts people. Never server names, addresses, user names, files, commands or anything you type.";

const EVERY_MS = 30 * 60 * 1000;

export const useStats = create<{ state: StatsState | null; load(): Promise<void>; set(on: boolean): Promise<void> }>((set) => ({
  state: null,
  async load() {
    try {
      set({ state: await api.statsState() });
    } catch {
      set({ state: { available: false, enabled: null } });
    }
  },
  async set(on) {
    const state = await api.setStats(on);
    set({ state });
    if (on) void api.statsPing("opened").catch(() => {});
  },
}));

/** Mounted once in App: loads the answer, then sends "opened" and "active" while the user said yes. */
export function useStatsPings() {
  const enabled = useStats((s) => s.state?.enabled === true && s.state.available);
  useEffect(() => { void useStats.getState().load(); }, []);
  useEffect(() => {
    if (!enabled) return;
    void api.statsPing("opened").catch(() => {});
    const t = setInterval(() => void api.statsPing("active").catch(() => {}), EVERY_MS);
    return () => clearInterval(t);
  }, [enabled]);
}
