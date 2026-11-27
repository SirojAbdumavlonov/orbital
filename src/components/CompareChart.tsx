import { ChevronDown, ChevronRight } from "lucide-react";
import { useEffect, useState } from "react";
import { api, MetricPoint, Profile } from "../lib/api";
import { bytes } from "../lib/format";
import { bounds, label, ticks, useRange } from "../lib/range";
import { paths } from "./HistoryChart";

// Home → Compare servers: one metric for several servers on one chart, over
// the shared date range, to see which one is busier and when. Up to six at
// once; the choice and whether it's open are kept on this computer.

type Metric = "cpu" | "mem" | "disk" | "load" | "rx";
const METRICS: { id: Metric; label: string }[] = [
  { id: "cpu", label: "CPU" },
  { id: "mem", label: "Memory" },
  { id: "disk", label: "Fullest disk" },
  { id: "load", label: "Load" },
  { id: "rx", label: "Network in" },
];
const COLORS = ["var(--log-1)", "var(--log-2)", "var(--log-3)", "var(--log-4)", "var(--log-5)", "var(--log-6)"];
const KEY = "orbital.compare";
const W = 720;
const H = 160;

/** The chart's top for this metric over these series: 100 for percentages, else the highest value. Exported for tests. */
export function topFor(metric: Metric, series: MetricPoint[][]): number {
  if (metric === "cpu" || metric === "mem" || metric === "disk") return 100;
  const vals = series.flat().map((p) => (p[metric] ?? 0) as number);
  return Math.max(metric === "load" ? 1 : 1024, ...vals);
}

function load(): { open: boolean; metric: Metric; ids: string[] } {
  try { return { open: false, metric: "cpu", ids: [], ...JSON.parse(localStorage.getItem(KEY) ?? "{}") }; } catch { return { open: false, metric: "cpu", ids: [] }; }
}

export function CompareServers({ profiles }: { profiles: Profile[] }) {
  const ssh = profiles.filter((p) => (p.protocol ?? "sftp") === "sftp");
  const [st, setSt] = useState(load);
  const save = (x: Partial<typeof st>) => setSt((s) => { const n = { ...s, ...x }; try { localStorage.setItem(KEY, JSON.stringify(n)); } catch { /* this run only */ } return n; });
  const ids = (st.ids.length ? st.ids : ssh.slice(0, 4).map((p) => p.id)).filter((id) => ssh.some((p) => p.id === id)).slice(0, 6);
  const range = useRange((x) => x.range);
  const [data, setData] = useState<Record<string, MetricPoint[]>>({});
  const [span, setSpan] = useState<{ from: number; to: number; bucket: number } | null>(null);
  useEffect(() => {
    if (!st.open) return;
    const { from, to } = bounds(range);
    Promise.all(ids.map((id) => api.metricHistory(id, from, to, 300).then(([b, pts]) => [id, b, pts] as const, () => [id, 60, []] as const))).then((rows) => {
      setData(Object.fromEntries(rows.map(([id, , pts]) => [id, [...pts]])));
      setSpan({ from, to, bucket: rows[0]?.[1] ?? 60 });
    });
  }, [st.open, JSON.stringify(range), ids.join(",")]);

  const series = ids.map((id) => data[id] ?? []);
  const top = topFor(st.metric, series);
  const fmt = (v: number) => (st.metric === "rx" ? `${bytes(v)}/s` : st.metric === "load" ? v.toFixed(1) : `${Math.round(v)}%`);
  const stat = (pts: MetricPoint[]) => {
    const v = pts.map((p) => p[st.metric]).filter((x): x is number => typeof x === "number");
    return v.length ? { avg: v.reduce((a, b) => a + b, 0) / v.length, peak: Math.max(...v) } : null;
  };
  if (ssh.length < 2) return null;
  return (
    <div className="compare">
      <button className="compare-head" onClick={() => save({ open: !st.open })} aria-expanded={st.open}>
        {st.open ? <ChevronDown size={14} /> : <ChevronRight size={14} />} Compare servers <span className="faint">{label(range).toLowerCase()}</span>
      </button>
      {st.open && (
        <div className="compare-body">
          <div className="compare-controls">
            <div className="segmented">
              {METRICS.map((m) => <button key={m.id} className={st.metric === m.id ? "on" : ""} onClick={() => save({ metric: m.id })}>{m.label}</button>)}
            </div>
            <div className="chips">
              {ssh.map((p) => {
                const on = ids.includes(p.id);
                return (
                  <button key={p.id} className={`chip ${on ? "on" : ""}`} disabled={!on && ids.length >= 6} title={!on && ids.length >= 6 ? "Up to six at once" : undefined}
                    onClick={() => save({ ids: on ? ids.filter((x) => x !== p.id) : [...ids, p.id] })}>
                    {on && <i className="compare-swatch" style={{ background: COLORS[ids.indexOf(p.id)] }} />}{p.name}
                  </button>
                );
              })}
            </div>
          </div>
          {span && (
            <svg viewBox={`0 0 ${W} ${H}`} width="100%" height={H} preserveAspectRatio="none" role="img" aria-label={`${METRICS.find((m) => m.id === st.metric)?.label} of ${ids.length} servers`}>
              {[0, 0.5, 1].map((f) => {
                const y = 10 + (1 - f) * (H - 28);
                return <g key={f}><line className="hist-grid" x1={66} x2={W - 8} y1={y} y2={y} /><text className="hist-label" x={62} y={y + 3} textAnchor="end">{fmt(top * f)}</text></g>;
              })}
              {ticks(span.from, span.to, 5).map((t, i) => (
                <text key={i} className="hist-label" x={66 + (i / 4) * (W - 74)} y={H - 4} textAnchor={i === 0 ? "start" : i === 4 ? "end" : "middle"}>{t.text}</text>
              ))}
              {series.map((pts, i) => (
                <path key={ids[i]} className="hist-line" style={{ stroke: COLORS[i] }} d={paths(pts, st.metric as "cpu", span.from, span.to, span.bucket * 3, top, H)} />
              ))}
            </svg>
          )}
          <table className="table compare-table">
            <thead><tr><th>Server</th><th className="num">Average</th><th className="num">Peak</th></tr></thead>
            <tbody>
              {ids.map((id, i) => {
                const p = ssh.find((x) => x.id === id)!;
                const s = stat(series[i]);
                return <tr key={id}><td><i className="compare-swatch" style={{ background: COLORS[i] }} />{p.name}</td><td className="num">{s ? fmt(s.avg) : "—"}</td><td className="num">{s ? fmt(s.peak) : "—"}</td></tr>;
              })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
