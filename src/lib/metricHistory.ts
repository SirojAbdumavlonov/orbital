import { useEffect, useState } from "react";
import { api, MetricPoint } from "./api";
import { bounds, Range, useRange } from "./range";

// Monitoring history comes from Orbital's database (metric_store.rs: a point
// a minute per server, kept 30 days, recorded as metrics are read). Views ask
// for the shared date range (lib/range.ts) and get it averaged into buckets.
// Pure parts (`diskForecast`, `toCsv`, `fromOld`) are tested.

export type HistoryPoint = MetricPoint;

/** History for the shared range, refreshed every minute. */
export function useRangeHistory(profileId: string | null | undefined, max = 720, override?: Range) {
  const shared = useRange((x) => x.range);
  const range = override ?? shared;
  const [data, setData] = useState<{ bucket: number; points: HistoryPoint[]; from: number; to: number } | null>(null);
  const key = JSON.stringify(range);
  useEffect(() => {
    if (!profileId) return;
    let alive = true;
    const load = () => {
      const { from, to } = bounds(range);
      api.metricHistory(profileId, from, to, max).then(([bucket, points]) => alive && setData({ bucket, points, from, to }), () => {});
    };
    load();
    const t = setInterval(load, 60_000);
    return () => { alive = false; clearInterval(t); };
  }, [profileId, key, max]);
  return data;
}

/**
 * When the fullest disk fills up at its recent pace: a straight line through
 * the points (least squares), from at least 12 hours of data, only when it's
 * rising. Days from the last point, or null.
 */
export function diskForecast(points: HistoryPoint[]): number | null {
  const pts = points.filter((p) => p.disk !== null) as (HistoryPoint & { disk: number })[];
  if (pts.length < 6 || pts[pts.length - 1].t - pts[0].t < 12 * 3600) return null;
  const n = pts.length;
  const mx = pts.reduce((a, p) => a + p.t, 0) / n;
  const my = pts.reduce((a, p) => a + p.disk, 0) / n;
  let num = 0, den = 0;
  for (const p of pts) { num += (p.t - mx) * (p.disk - my); den += (p.t - mx) ** 2; }
  if (den === 0) return null;
  const slope = num / den; // % per second
  if (slope <= 0) return null;
  const last = pts[n - 1];
  const now = my + slope * (last.t - mx);
  const days = (100 - now) / slope / 86400;
  return days > 0 && days < 365 ? Math.round(days * 10) / 10 : null;
}

/** The points as CSV (times in ISO 8601, UTC). */
export function toCsv(points: HistoryPoint[]): string {
  const rows = ["time,cpu_percent,memory_percent,fullest_disk_percent,load_1m,net_in_bytes_per_sec,net_out_bytes_per_sec"];
  const v = (x: number | null | undefined) => (x === null || x === undefined ? "" : String(x));
  for (const p of points) rows.push([new Date(p.t * 1000).toISOString(), v(p.cpu), v(p.mem), v(p.disk), v(p.load), v(p.rx), v(p.tx)].join(","));
  return rows.join("\n") + "\n";
}

/** A point kept by versions up to 1.9 (CPU, memory and disk only). */
export function fromOld(p: any): HistoryPoint | null {
  if (!p || typeof p.t !== "number" || typeof p.mem !== "number") return null;
  return { t: p.t, cpu: typeof p.cpu === "number" ? p.cpu : null, mem: p.mem, disk: typeof p.disk === "number" ? p.disk : null, load: 0, rx: null, tx: null, uptime: null };
}

/** Mounted once in App: moves history kept in local storage by older versions into the database, then drops it. */
export function useHistoryMigration() {
  useEffect(() => {
    let keys: string[] = [];
    try { keys = Object.keys(localStorage).filter((k) => k.startsWith("orbital.history.")); } catch { return; }
    for (const k of keys) {
      let pts: HistoryPoint[] = [];
      try { pts = (JSON.parse(localStorage.getItem(k) ?? "[]") as any[]).map(fromOld).filter(Boolean) as HistoryPoint[]; } catch { /* unreadable: drop it */ }
      const pid = k.slice("orbital.history.".length);
      const done = () => { try { localStorage.removeItem(k); } catch { /* next time */ } };
      if (!pts.length) { done(); continue; }
      api.importMetricHistory(pid, pts).then(done, () => {});
    }
  }, []);
}
