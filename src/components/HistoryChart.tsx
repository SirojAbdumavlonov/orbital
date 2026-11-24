import { useEffect } from "react";
import { DAY, historyOf, HistoryPoint, useHistory } from "../lib/metricHistory";

// The last 24 hours of a server: CPU, memory and fullest disk (0–100 %), one
// point a minute. A gap of more than 3 minutes (Orbital closed or not
// connected) breaks the line instead of drawing across it.

const W = 720;
const H = 140;
const PAD = { l: 30, r: 8, t: 8, b: 18 };
const SERIES: { key: "cpu" | "mem" | "disk"; label: string; cls: string }[] = [
  { key: "cpu", label: "CPU", cls: "hist-cpu" },
  { key: "mem", label: "Memory", cls: "hist-mem" },
  { key: "disk", label: "Fullest disk", cls: "hist-disk" },
];

/** SVG path segments for one series, broken at gaps. Exported for tests. */
export function paths(points: HistoryPoint[], key: "cpu" | "mem" | "disk", from: number, to: number): string {
  const x = (t: number) => PAD.l + ((t - from) / (to - from)) * (W - PAD.l - PAD.r);
  const y = (v: number) => PAD.t + (1 - Math.min(100, Math.max(0, v)) / 100) * (H - PAD.t - PAD.b);
  let d = "";
  let prev: number | null = null;
  for (const p of points) {
    const v = p[key];
    if (v === null || p.t < from) { prev = null; continue; }
    d += `${prev === null || p.t - prev > 180 ? "M" : "L"}${x(p.t).toFixed(1)},${y(v).toFixed(1)}`;
    prev = p.t;
  }
  return d;
}

export function HistoryChart({ profileId }: { profileId: string }) {
  const pts = useHistory((h) => h.byProfile[profileId]) ?? [];
  useEffect(() => { historyOf(profileId); }, [profileId]);
  const to = Math.floor(Date.now() / 60_000) * 60;
  const from = to - DAY;
  const last = pts[pts.length - 1];
  const hours = [24, 18, 12, 6, 0];
  return (
    <div className="hist">
      <div className="hist-legend">
        {SERIES.map((s) => (
          <span key={s.key}><i className={s.cls} /> {s.label}{last && last[s.key] !== null ? ` ${Math.round(last[s.key]!)}%` : ""}</span>
        ))}
        <span className="faint">{pts.length ? (pts.length < 60 ? `${pts.length} min recorded` : `${Math.round((pts.length / 60) * 10) / 10} h recorded`) : "Recorded while Orbital is connected; nothing yet"}</span>
      </div>
      <svg viewBox={`0 0 ${W} ${H}`} width="100%" height={H} preserveAspectRatio="none" role="img" aria-label="CPU, memory and disk over the last 24 hours">
        {[0, 25, 50, 75, 100].map((v) => {
          const yy = PAD.t + (1 - v / 100) * (H - PAD.t - PAD.b);
          return (
            <g key={v}>
              <line className="hist-grid" x1={PAD.l} x2={W - PAD.r} y1={yy} y2={yy} />
              <text className="hist-label" x={PAD.l - 4} y={yy + 3} textAnchor="end">{v}</text>
            </g>
          );
        })}
        {hours.map((h) => (
          <text key={h} className="hist-label" x={PAD.l + ((24 - h) / 24) * (W - PAD.l - PAD.r)} y={H - 4} textAnchor={h === 24 ? "start" : h === 0 ? "end" : "middle"}>
            {h === 0 ? "now" : `−${h} h`}
          </text>
        ))}
        {SERIES.map((s) => <path key={s.key} className={`hist-line ${s.cls}`} d={paths(pts, s.key, from, to)} />)}
      </svg>
    </div>
  );
}
