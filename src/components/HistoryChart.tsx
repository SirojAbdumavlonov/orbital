import { Download, Pin, PinOff } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { api, asApiError, MetricEvent, Moment } from "../lib/api";
import { bytes } from "../lib/format";
import { HistoryPoint, toCsv, useRangeHistory } from "../lib/metricHistory";
import { label, ticks, useRange } from "../lib/range";
import { useStore } from "../store";

// A server's history over the shared date range (lib/range.ts): CPU, memory
// and fullest disk (0–100 %), then network in and out. Points are averaged
// into buckets by the engine; a gap of more than three buckets (Orbital
// closed or not connected) breaks the line instead of drawing across it.
// Hover (or click to pin) a moment: its values, the changes Orbital made then
// and the busiest programs at that minute. Changes and restarts are marked
// along the top of the chart.

const W = 720;
const H = 140;
const PAD = { l: 66, r: 8, t: 10, b: 18 };
type Key = "cpu" | "mem" | "disk" | "rx" | "tx";
const PCT: { key: Key; label: string; cls: string }[] = [
  { key: "cpu", label: "CPU", cls: "hist-cpu" },
  { key: "mem", label: "Memory", cls: "hist-mem" },
  { key: "disk", label: "Fullest disk", cls: "hist-disk" },
];
const NET: { key: Key; label: string; cls: string }[] = [
  { key: "rx", label: "In", cls: "hist-mem" },
  { key: "tx", label: "Out", cls: "hist-cpu" },
];

const xOf = (t: number, from: number, to: number) => PAD.l + ((t - from) / (to - from || 1)) * (W - PAD.l - PAD.r);

/** SVG path segments for one series, broken at gaps. Exported for tests. */
export function paths(points: Partial<HistoryPoint>[], key: Key, from: number, to: number, gap = 180, top = 100, h = H): string {
  const y = (v: number) => PAD.t + (1 - Math.min(top, Math.max(0, v)) / (top || 1)) * (h - PAD.t - PAD.b);
  let d = "";
  let prev: number | null = null;
  for (const p of points) {
    const v = p[key] as number | null | undefined;
    if (v === null || v === undefined || p.t === undefined || p.t < from) { prev = null; continue; }
    d += `${prev === null || p.t - prev > gap ? "M" : "L"}${xOf(p.t, from, to).toFixed(1)},${y(v).toFixed(1)}`;
    prev = p.t;
  }
  return d;
}

/** The point nearest `t`, if one is within `gap` seconds. Exported for tests. */
export function nearest<T extends { t: number }>(points: T[], t: number, gap: number): T | null {
  let best: T | null = null;
  for (const p of points) if (!best || Math.abs(p.t - t) < Math.abs(best.t - t)) best = p;
  return best && Math.abs(best.t - t) <= gap ? best : null;
}

/** Changes and restarts in one bucket around `t`. Exported for tests. */
export function eventsNear(events: MetricEvent[], t: number, bucket: number): MetricEvent[] {
  return events.filter((e) => e.t >= t - bucket / 2 && e.t < t + bucket / 2 + 60);
}

/** "Restarted nginx", "Saved /etc/nginx/nginx.conf"… from an audit action. */
export function describeEvent(e: MetricEvent): string {
  if (e.kind === "restart") return "Server restarted";
  const verb = e.action.split(".").slice(1).join(" ").replace(/_/g, " ") || e.action;
  const what = e.target ? ` ${e.target}` : "";
  return `${verb.charAt(0).toUpperCase()}${verb.slice(1)}${what}${e.ok ? "" : " (failed)"}`;
}

function Chart({ points, from, to, gap, series, top, fmt, h, aria, hoverT, events, onPick }: {
  points: HistoryPoint[]; from: number; to: number; gap: number; series: typeof PCT; top: number; fmt: (v: number) => string; h: number; aria: string;
  hoverT: number | null; events: MetricEvent[]; onPick: (t: number | null, click: boolean) => void;
}) {
  const tk = ticks(from, to, 5);
  const ref = useRef<SVGSVGElement>(null);
  const tAt = (clientX: number) => {
    const r = ref.current?.getBoundingClientRect();
    if (!r) return null;
    const x = ((clientX - r.left) / r.width) * W;
    if (x < PAD.l || x > W - PAD.r) return null;
    return from + ((x - PAD.l) / (W - PAD.l - PAD.r)) * (to - from);
  };
  return (
    <svg ref={ref} viewBox={`0 0 ${W} ${h}`} width="100%" height={h} preserveAspectRatio="none" role="img" aria-label={aria}
      className="hist-svg" onMouseMove={(e) => onPick(tAt(e.clientX), false)} onMouseLeave={() => onPick(null, false)} onClick={(e) => onPick(tAt(e.clientX), true)}>
      {[0, 0.5, 1].map((f) => {
        const yy = PAD.t + (1 - f) * (h - PAD.t - PAD.b);
        return (
          <g key={f}>
            <line className="hist-grid" x1={PAD.l} x2={W - PAD.r} y1={yy} y2={yy} />
            <text className="hist-label" x={PAD.l - 4} y={yy + 3} textAnchor="end">{fmt(top * f)}</text>
          </g>
        );
      })}
      {tk.map((t, i) => (
        <text key={i} className="hist-label" x={PAD.l + (i / (tk.length - 1)) * (W - PAD.l - PAD.r)} y={h - 4} textAnchor={i === 0 ? "start" : i === tk.length - 1 ? "end" : "middle"}>{t.text}</text>
      ))}
      {events.map((e, i) => (
        <line key={i} className={`hist-event ${e.kind === "restart" ? "restart" : e.ok ? "" : "failed"}`} x1={xOf(e.t, from, to)} x2={xOf(e.t, from, to)} y1={PAD.t - 8} y2={h - PAD.b}>
          <title>{`${new Date(e.t * 1000).toLocaleString()}: ${describeEvent(e)}`}</title>
        </line>
      ))}
      {series.map((s) => <path key={s.key} className={`hist-line ${s.cls}`} d={paths(points, s.key, from, to, gap, top, h)} />)}
      {hoverT !== null && <line className="hist-cross" x1={xOf(hoverT, from, to)} x2={xOf(hoverT, from, to)} y1={PAD.t} y2={h - PAD.b} />}
    </svg>
  );
}

const rate = (v: number) => `${bytes(v)}/s`;
const fmtPct = (v: number | null | undefined) => (v === null || v === undefined ? "—" : `${Math.round(v)}%`);

/** What was running at that minute (fetched once per minute hovered). */
function useMoment(profileId: string, t: number | null, bucket: number) {
  const [m, setM] = useState<{ t: number; m: Moment | null } | null>(null);
  const cache = useRef(new Map<number, Moment | null>());
  useEffect(() => {
    if (t === null) return;
    const key = Math.round(t / 60) * 60;
    if (cache.current.has(key)) { setM({ t: key, m: cache.current.get(key)! }); return; }
    const h = setTimeout(() => {
      api.metricMoment(profileId, key, Math.max(120, bucket)).then((x) => { cache.current.set(key, x); setM({ t: key, m: x }); }, () => {});
    }, 200);
    return () => clearTimeout(h);
  }, [profileId, t === null ? null : Math.round(t / 60), bucket]);
  return m;
}

export function HistoryChart({ profileId, name }: { profileId: string; name: string }) {
  const range = useRange((x) => x.range);
  const data = useRangeHistory(profileId);
  const toast = useStore((x) => x.toast);
  const pts = data?.points ?? [];
  const from = data?.from ?? 0;
  const to = data?.to ?? 1;
  const bucket = data?.bucket ?? 60;
  const gap = bucket * 3;
  const [events, setEvents] = useState<MetricEvent[]>([]);
  useEffect(() => {
    if (!data) return;
    api.metricEvents(profileId, data.from, data.to).then(setEvents, () => setEvents([]));
  }, [profileId, data?.from, data?.to]);
  const [hover, setHover] = useState<number | null>(null);
  const [pinned, setPinned] = useState<number | null>(null);
  const at = pinned ?? hover;
  const point = at === null ? null : nearest(pts, at, gap);
  const near = useMemo(() => (point ? eventsNear(events, point.t, bucket) : []), [point?.t, events, bucket]);
  const moment = useMoment(profileId, point ? point.t + bucket / 2 : null, bucket);
  const pick = (t: number | null, click: boolean) => {
    if (click) setPinned((p) => (p !== null || t === null ? null : t));
    else setHover(t);
  };
  useEffect(() => { setPinned(null); }, [JSON.stringify(range)]);

  const netTop = Math.max(1024, ...pts.map((p) => Math.max(p.rx ?? 0, p.tx ?? 0)));
  const avg = (k: "cpu" | "mem") => {
    const v = pts.map((p) => p[k]).filter((x): x is number => x !== null);
    return v.length ? Math.round(v.reduce((a, b) => a + b, 0) / v.length) : null;
  };
  const peak = (k: "cpu" | "mem" | "disk") => {
    const v = pts.map((p) => p[k]).filter((x): x is number => x !== null);
    return v.length ? Math.round(Math.max(...v)) : null;
  };
  const exportCsv = async () => {
    try {
      const { save: saveDialog } = await import("@tauri-apps/plugin-dialog");
      const path = await saveDialog({ defaultPath: `${name}-history.csv`, filters: [{ name: "CSV", extensions: ["csv"] }] });
      if (!path) return;
      await api.saveText(path, toCsv(pts));
      toast({ kind: "success", title: "History saved", body: path });
    } catch (e) {
      toast({ kind: "error", title: "Couldn't save the history", body: asApiError(e).message });
    }
  };
  const chartProps = { points: pts, from, to, gap, hoverT: point ? point.t : null, events, onPick: pick };
  const changes = events.filter((e) => e.kind !== "restart").length;
  const restarts = events.length - changes;
  return (
    <div className="hist">
      <div className="hist-legend">
        {PCT.map((s) => (
          <span key={s.key}><i className={s.cls} /> {s.label}{s.key !== "disk" && avg(s.key as "cpu" | "mem") !== null ? ` avg ${avg(s.key as "cpu" | "mem")}%` : ""}{peak(s.key as "cpu") !== null ? ` · peak ${peak(s.key as "cpu")}%` : ""}</span>
        ))}
        {events.length > 0 && <span title="Marked along the top of the chart"><i className="hist-event-key" /> {[changes ? `${changes} change${changes === 1 ? "" : "s"}` : "", restarts ? `${restarts} restart${restarts === 1 ? "" : "s"}` : ""].filter(Boolean).join(", ")}</span>}
        <span className="faint">{pts.length ? `${label(range)}${bucket > 60 ? ` · a point per ${bucket >= 3600 ? `${Math.round(bucket / 360) / 10} h` : `${bucket / 60} min`}` : ""}` : "Recorded while Orbital is connected; nothing in this range"}</span>
        <button className="icon-btn" title="Save this range as CSV" aria-label="Export CSV" disabled={!pts.length} onClick={() => void exportCsv()}><Download size={14} /></button>
      </div>
      <Chart {...chartProps} series={PCT} top={100} fmt={(v) => `${Math.round(v)}`} h={H} aria={`CPU, memory and disk, ${label(range)}`} />
      <div className="hist-legend">
        {NET.map((s) => <span key={s.key}><i className={s.cls} /> Network {s.label.toLowerCase()}</span>)}
      </div>
      <Chart {...chartProps} series={NET} top={netTop} fmt={(v) => (v ? rate(v) : "0")} h={90} aria={`Network in and out, ${label(range)}`} />
      <div className={`hist-moment ${point ? "" : "empty"}`}>
        {!point ? (
          <span className="faint">Point at the chart to see that moment: its numbers, what changed and the busiest programs. Click to keep it.</span>
        ) : (
          <>
            <div className="hist-moment-head">
              <b>{new Date(point.t * 1000).toLocaleString(undefined, { weekday: "short", day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" })}{bucket > 60 ? ` (${bucket >= 3600 ? `${Math.round(bucket / 360) / 10} h` : `${bucket / 60} min`} average)` : ""}</b>
              <span>CPU {fmtPct(point.cpu)} · Memory {fmtPct(point.mem)} · Disk {fmtPct(point.disk)} · Load {point.load.toFixed(2)} · In {point.rx === null ? "—" : rate(point.rx)} · Out {point.tx === null ? "—" : rate(point.tx)}</span>
              <button className="icon-btn" title={pinned !== null ? "Let go" : "Keep this moment"} aria-label={pinned !== null ? "Unpin" : "Pin"} onClick={() => setPinned(pinned !== null ? null : point.t)}>{pinned !== null ? <PinOff size={13} /> : <Pin size={13} />}</button>
            </div>
            {near.length > 0 && (
              <ul className="hist-moment-events">
                {near.map((e, i) => <li key={i} className={e.kind === "restart" ? "restart" : e.ok ? "" : "failed"}>{new Date(e.t * 1000).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })} · {describeEvent(e)}</li>)}
              </ul>
            )}
            {moment?.m?.procs.length ? (
              <table className="table hist-procs">
                <thead><tr><th>Busiest programs then</th><th>User</th><th className="num">CPU</th><th className="num">Memory</th></tr></thead>
                <tbody>
                  {[...moment.m.procs].sort((a, b) => b.cpu - a.cpu).slice(0, 6).map((p) => (
                    <tr key={p.pid}><td className="mono">{p.name} <span className="faint">{p.pid}</span></td><td>{p.user}</td><td className="num">{p.cpu.toFixed(1)}%</td><td className="num">{p.mem.toFixed(1)}% · {bytes(p.rssKb * 1024)}</td></tr>
                  ))}
                </tbody>
              </table>
            ) : moment ? <span className="faint">No programs recorded for that minute.</span> : null}
          </>
        )}
      </div>
    </div>
  );
}
