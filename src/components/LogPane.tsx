import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { CalendarClock, Download, Filter, Play, X } from "lucide-react";
import { ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { api, asApiError, LogSource } from "../lib/api";
import { filterLines, keepTail } from "../lib/logs";
import { useStore } from "../store";

/** From/to as `datetime-local` values (this computer's time zone); either may be empty. */
export type LogRange = { from: string; to: string };

/** Unix seconds, or null for an empty field. */
export const rangeSeconds = (v: string) => (v ? Math.floor(new Date(v).getTime() / 1000) : null);
export const rangeIso = (v: string) => (v ? new Date(v).toISOString() : null);

const fmtLocal = (d: Date) => new Date(d.getTime() - d.getTimezoneOffset() * 60_000).toISOString().slice(0, 16);
const shortTime = (v: string) => new Date(v).toLocaleString([], { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
function rangeLabel(r: LogRange) {
  return r.from && r.to ? `${shortTime(r.from)} – ${shortTime(r.to)}` : r.from ? `since ${shortTime(r.from)}` : `until ${shortTime(r.to)}`;
}

/**
 * A log pane: the recent lines, then new ones live (`follow`), or what was
 * written between two times (`load(range)`); search and "errors only" on top.
 * Returns the header buttons and the body, so it fits a VSplit pane or a drawer.
 */
export function useLogPane({ sid, id, follow, load, name }: {
  sid: string;
  /** What is shown (container id, unit); null shows nothing. */
  id: string | null;
  follow: LogSource | null;
  load: (range: LogRange | null) => Promise<string>;
  /** For the saved file's name. */
  name: string;
}): { text: string; extra: ReactNode; body: ReactNode } {
  const toast = useStore((st) => st.toast);
  const [text, setText] = useState("");
  const [live, setLive] = useState(true);
  const [range, setRange] = useState<LogRange | null>(null);
  const [rangeOpen, setRangeOpen] = useState(false);
  const [filterOpen, setFilterOpen] = useState(false);
  const [q, setQ] = useState("");
  const [problems, setProblems] = useState(false);
  const fetched = useRef<string | null>(null);
  const box = useRef<HTMLPreElement>(null);
  const stick = useRef(true);

  useEffect(() => { setRange(null); setRangeOpen(false); }, [id]);
  useEffect(() => {
    if (!id) return;
    let gone = false;
    let stream: string | null = null;
    const key = `${id}|${range?.from}|${range?.to}`;
    (async () => {
      if (fetched.current !== key) {
        stick.current = true;
        setText("Loading…");
        const t = await load(range);
        if (gone) return;
        fetched.current = key;
        setText(range && !t.trim() ? "Nothing was written between these times." : t);
      }
      if (!live || range || !follow) return;
      const s = await api.followLogs(sid, follow, 0, (m) => {
        if (m.type === "data") setText((x) => keepTail(x + m.text));
      });
      if (gone) api.stopLogs(s).catch(() => {});
      else stream = s;
    })().catch((e) => !gone && setText((x) => (x === "Loading…" ? asApiError(e).message : x)));
    return () => {
      gone = true;
      if (stream) api.stopLogs(stream).catch(() => {});
    };
    // `follow` and `load` are rebuilt every render; `id` stands for them.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sid, id, live, range]);
  useEffect(() => {
    if (stick.current && box.current) box.current.scrollTop = box.current.scrollHeight;
  }, [text, q, problems]);

  const lines = useMemo(() => filterLines(text, q, problems), [text, q, problems]);
  const filtering = !!q.trim() || problems;

  function toggleLive() {
    if (!live) fetched.current = null;
    setLive(!live);
  }
  async function save() {
    const path = await saveDialog({ defaultPath: `${name}-${new Date().toISOString().slice(0, 16).replace(/[:T]/g, "-")}.log`, title: "Save log" });
    if (!path) return;
    try {
      await api.saveText(path, lines.map((l) => l.text).join("\n") + "\n");
      toast({ kind: "success", title: "Log saved", body: path });
    } catch (e) {
      toast({ kind: "error", title: "Couldn't save the log", body: asApiError(e).message });
    }
  }

  const extra = (
    <>
      <button className={`icon-btn ${filterOpen || filtering ? "on" : ""}`} aria-label="Search the log" title="Search, or show only errors and warnings" onClick={() => setFilterOpen(!filterOpen)}><Filter size={13} /></button>
      <button className={`icon-btn ${rangeOpen || range ? "on" : ""}`} aria-label="Logs between two times" title="Show what was written between two times" onClick={() => setRangeOpen(!rangeOpen)}><CalendarClock size={13} /></button>
      <button className="icon-btn" aria-label="Save the log" title="Save the lines shown to a file" onClick={save}><Download size={13} /></button>
      {range ? (
        <button className="live-toggle" title="Back to the latest lines" onClick={() => { setRange(null); setRangeOpen(false); }}>{rangeLabel(range)} <X size={11} /></button>
      ) : follow && (
        <button className={`live-toggle ${live ? "on" : ""}`} title={live ? "New lines appear as they're written. Click to pause." : "Paused. Click to follow new lines again."} onClick={toggleLive}>
          {live ? <><span className="live-dot" /> Live</> : <><Play size={11} /> Paused</>}
        </button>
      )}
    </>
  );

  const body = (
    <>
      {rangeOpen && <RangeForm initial={range} onApply={(r) => { setRange(r); setRangeOpen(false); }} onCancel={() => setRangeOpen(false)} />}
      {filterOpen && (
        <div className="log-filter">
          <input className="input" autoFocus placeholder="Show lines containing…" value={q} onChange={(e) => setQ(e.target.value)} onKeyDown={(e) => e.key === "Escape" && (setQ(""), setFilterOpen(false))} />
          <label className="check"><input type="checkbox" checked={problems} onChange={(e) => setProblems(e.target.checked)} /> Errors and warnings only</label>
          {filtering && <span className="faint">{lines.length} lines</span>}
        </div>
      )}
      <pre className="logs" ref={box} onScroll={(e) => { const el = e.currentTarget; stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40; }}>
        {filtering && lines.length === 0 ? <span className="faint">No lines match.</span> : lines.map((l, i) => <span key={i} className={l.level ? `lv-${l.level}` : undefined}>{l.text}{"\n"}</span>)}
      </pre>
    </>
  );
  return { text, extra, body };
}

/** From/to pickers (this computer's time zone) with one-click recent windows. */
function RangeForm({ initial, onApply, onCancel }: { initial: LogRange | null; onApply: (r: LogRange) => void; onCancel: () => void }) {
  const [from, setFrom] = useState(initial?.from ?? fmtLocal(new Date(Date.now() - 3600_000)));
  const [to, setTo] = useState(initial?.to ?? "");
  const bad = !from && !to ? "Pick at least one of the two times." : from && to && new Date(from) >= new Date(to) ? "“From” has to be before “to”." : null;
  const last = (min: number) => onApply({ from: fmtLocal(new Date(Date.now() - min * 60_000)), to: "" });
  return (
    <form className="log-range" onSubmit={(e) => { e.preventDefault(); if (!bad) onApply({ from, to }); }}>
      <div className="log-range-quick">
        {([[15, "15 min"], [60, "1 hour"], [360, "6 hours"], [1440, "24 hours"]] as const).map(([m, l]) => (
          <button type="button" key={m} className="chip" onClick={() => last(m)}>Last {l}</button>
        ))}
      </div>
      <label>From <input className="input" type="datetime-local" value={from} onChange={(e) => setFrom(e.target.value)} /></label>
      <label>To <input className="input" type="datetime-local" value={to} onChange={(e) => setTo(e.target.value)} /></label>
      <div className="log-range-foot">
        <span className="faint">{bad ?? "Your computer's time zone. Leave “to” empty for now. Up to 5,000 lines."}</span>
        <span className="spacer" />
        <button type="button" className="btn" onClick={onCancel}>Cancel</button>
        <button className="btn primary" disabled={!!bad}>Show</button>
      </div>
    </form>
  );
}
