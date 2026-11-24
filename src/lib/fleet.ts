import { create } from "zustand";
import { useEffect } from "react";
import { api, Certificate, UpdatesReport } from "./api";
import { isSsh, useStore } from "../store";

// Pending updates on every connected server, for the Home cards: checked when
// a server connects and then every hour (read-only: the package list the
// server already has; refreshing it is left to the Updates tab).

export interface FleetUpdates {
  count: number;
  security: number;
  reboot: boolean;
  /** Unix ms of the check. */
  at: number;
}

export function summarize(r: UpdatesReport): FleetUpdates {
  return { count: r.packages.length, security: r.packages.filter((p) => p.security).length, reboot: r.rebootRequired, at: Date.now() };
}

/** "12 updates (3 security) · reboot needed", or null when there's nothing to say. */
export function updatesText(u: FleetUpdates | undefined): string | null {
  if (!u || (!u.count && !u.reboot)) return null;
  const parts: string[] = [];
  if (u.count) parts.push(`${u.count} update${u.count === 1 ? "" : "s"}${u.security ? ` (${u.security} security)` : ""}`);
  if (u.reboot) parts.push("reboot needed");
  return parts.join(" · ");
}

/** Certificates that expire within three weeks, or already have, soonest first. */
export function expiring(certs: Certificate[], days = 21): { name: string; daysLeft: number }[] {
  return certs
    .filter((c) => c.daysLeft !== null && c.daysLeft <= days)
    .map((c) => ({ name: c.domains[0] ?? c.name, daysLeft: c.daysLeft as number }))
    .sort((a, b) => a.daysLeft - b.daysLeft);
}

/** "HTTPS for shop.example.com expires in 9 days (+1 more)", or null. */
export function certText(list: { name: string; daysLeft: number }[] | undefined): string | null {
  if (!list || !list.length) return null;
  const c = list[0];
  const when = c.daysLeft < 0 ? `expired ${-c.daysLeft} day${c.daysLeft === -1 ? "" : "s"} ago` : c.daysLeft === 0 ? "expires today" : `expires in ${c.daysLeft} day${c.daysLeft === 1 ? "" : "s"}`;
  return `HTTPS for ${c.name} ${when}${list.length > 1 ? ` (+${list.length - 1} more)` : ""}`;
}

export const useFleet = create<{ updates: Record<string, FleetUpdates>; certs: Record<string, { name: string; daysLeft: number }[]> }>(() => ({ updates: {}, certs: {} }));

const EVERY_MS = 60 * 60 * 1000;

/** Mounted once in App. */
export function useFleetUpdates() {
  useEffect(() => {
    let stop = false;
    const checked: Record<string, number> = {};
    const tick = async () => {
      const sessions = Object.values(useStore.getState().sessions).filter((s) => !s.lost && isSsh(s));
      for (const s of sessions) {
        if (stop) return;
        if (checked[s.info.id] && Date.now() - checked[s.info.id] < EVERY_MS) continue;
        checked[s.info.id] = Date.now();
        try {
          const u = summarize(await api.updates(s.info.id));
          useFleet.setState((f) => ({ updates: { ...f.updates, [s.profile.id]: u } }));
        } catch {
          // No package manager, or not reachable now: try again in an hour.
        }
        // Certificates (certbot), wherever certbot is installed.
        if (s.tools?.some((t) => t.id === "certbot")) {
          try {
            const c = expiring((await api.certbotStatus(s.info.id)).certificates);
            useFleet.setState((f) => ({ certs: { ...f.certs, [s.profile.id]: c } }));
          } catch { /* certbot busy or unreadable: next hour */ }
        }
      }
    };
    void tick();
    const t = setInterval(tick, 60_000);
    // A server that just connected is checked a few seconds later, not at the next minute.
    let count = Object.keys(useStore.getState().sessions).length;
    let soon: number | undefined;
    const unsub = useStore.subscribe((st) => {
      const n = Object.keys(st.sessions).length;
      if (n > count) { clearTimeout(soon); soon = window.setTimeout(tick, 4000); }
      count = n;
    });
    return () => { stop = true; clearInterval(t); clearTimeout(soon); unsub(); };
  }, []);
}
