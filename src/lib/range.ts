import { create } from "zustand";

// One date range for every monitoring dashboard: the Monitor tab's history,
// the Home cards' trends and the Uptime page all follow it, so changing it in
// one place changes it everywhere. Kept on this computer
// (localStorage `orbital.range`). Pure parts are tested.

export type Preset = "1h" | "6h" | "24h" | "7d" | "30d";
export type Range = { preset: Preset } | { from: number; to: number };

export const PRESETS: { id: Preset; label: string; secs: number }[] = [
  { id: "1h", label: "1 h", secs: 3600 },
  { id: "6h", label: "6 h", secs: 6 * 3600 },
  { id: "24h", label: "24 h", secs: 24 * 3600 },
  { id: "7d", label: "7 days", secs: 7 * 86400 },
  { id: "30d", label: "30 days", secs: 30 * 86400 },
];

/** The range as Unix seconds; presets end now. */
export function bounds(r: Range, nowSecs = Math.floor(Date.now() / 1000)): { from: number; to: number } {
  if ("preset" in r) {
    const p = PRESETS.find((x) => x.id === r.preset) ?? PRESETS[2];
    return { from: nowSecs - p.secs, to: nowSecs };
  }
  return { from: Math.min(r.from, r.to), to: Math.max(r.from, r.to) };
}

/** "Last 24 h", or "3 Nov 10:00 – 5 Nov 18:30". */
export function label(r: Range): string {
  if ("preset" in r) return `Last ${PRESETS.find((x) => x.id === r.preset)?.label ?? "24 h"}`;
  const f = (s: number) => new Date(s * 1000).toLocaleString(undefined, { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
  const { from, to } = bounds(r);
  return `${f(from)} – ${f(to)}`;
}

/** Axis ticks: about `n` evenly spaced times with short labels fitting the span. */
export function ticks(from: number, to: number, n = 5): { t: number; text: string }[] {
  const span = to - from;
  const out: { t: number; text: string }[] = [];
  for (let i = 0; i < n; i++) {
    const t = from + (span * i) / (n - 1);
    const d = new Date(t * 1000);
    const text = span <= 2 * 86400
      ? d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })
      : d.toLocaleDateString(undefined, { day: "numeric", month: "short" });
    out.push({ t, text });
  }
  return out;
}

/** `YYYY-MM-DDTHH:MM` in local time, for <input type="datetime-local">, and back. */
export const toLocalInput = (secs: number) => {
  const d = new Date(secs * 1000);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}`;
};
export const fromLocalInput = (v: string): number | null => {
  const t = new Date(v).getTime();
  return Number.isFinite(t) ? Math.floor(t / 1000) : null;
};

const KEY = "orbital.range";
function loadRange(): Range {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "null");
    if (v && typeof v === "object" && ("preset" in v ? PRESETS.some((p) => p.id === v.preset) : typeof v.from === "number" && typeof v.to === "number")) return v;
  } catch { /* default */ }
  return { preset: "24h" };
}

export const useRange = create<{ range: Range; set(r: Range): void }>((set) => ({
  range: loadRange(),
  set(range) {
    try { localStorage.setItem(KEY, JSON.stringify(range)); } catch { /* this run only */ }
    set({ range });
  },
}));
