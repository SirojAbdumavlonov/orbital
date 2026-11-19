import { Eraser, Pause, Play, Plus, Search, WrapText, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { api, asApiError, Container, LogSource } from "../lib/api";
import { bytes } from "../lib/format";
import { SessionState, useStore } from "../store";
import { level } from "../lib/logs";

const MAX_LINES = 5000;
const COLORS = [1, 2, 3, 4, 5, 6].map((n) => `var(--log-${n})`);

interface Line { n: number; src: number; text: string }
interface Followed { key: string; label: string; source: LogSource; id: string | null; color: string; ended: boolean }

const keyOf = (s: LogSource) => (s.type === "file" ? `f:${s.path}` : s.type === "unit" ? `u:${s.unit}` : `c:${s.id}`);
const labelOf = (s: LogSource) => (s.type === "file" ? s.path.replace(/^\/var\/log\//, "") : s.type === "unit" ? s.unit.replace(/\.service$/, "") : s.id);


function SourcePicker({ s, onPick, onClose }: { s: SessionState; onPick: (src: LogSource) => void; onClose: () => void }) {
  const [files, setFiles] = useState<{ path: string; size: number }[] | null>(null);
  const [units, setUnits] = useState<string[]>([]);
  const [containers, setContainers] = useState<Container[]>([]);
  const [q, setQ] = useState("");
  useEffect(() => {
    api.logFiles(s.info.id).then(setFiles).catch(() => setFiles([]));
    if (s.info.hasSystemd) api.listServices(s.info.id).then((l) => setUnits(l.filter((u) => u.active === "active" || u.active === "failed").map((u) => u.unit))).catch(() => {});
    if (s.info.dockerVersion) api.listContainers(s.info.id).then(setContainers).catch(() => {});
  }, [s.info.id]);
  const match = (t: string) => !q || t.toLowerCase().includes(q.toLowerCase());
  return (
    <div className="log-picker">
      <div className="search" style={{ padding: 8 }}><Search size={14} /><input className="input" autoFocus placeholder="Find a log, service or container" value={q} onChange={(e) => setQ(e.target.value)} />
        <button className="icon-btn" aria-label="Close" onClick={onClose}><X size={14} /></button>
      </div>
      <div className="scroll" style={{ maxHeight: 340 }}>
        {units.filter(match).length > 0 && <div className="section-title">Services (journal)</div>}
        {units.filter(match).map((u) => <button key={u} className="menu-item" onClick={() => onPick({ type: "unit", unit: u })}><span className="mono">{u.replace(/\.service$/, "")}</span></button>)}
        {containers.filter((c) => match(c.names)).length > 0 && <div className="section-title">Containers</div>}
        {containers.filter((c) => match(c.names)).map((c) => <button key={c.id} className="menu-item" onClick={() => onPick({ type: "container", id: c.names })}><span className="mono">{c.names}</span><span className="faint" style={{ marginLeft: "auto" }}>{c.state}</span></button>)}
        <div className="section-title">Log files</div>
        {files === null && <div className="menu-empty">Looking in /var/log…</div>}
        {files?.filter((f) => match(f.path)).map((f) => <button key={f.path} className="menu-item" onClick={() => onPick({ type: "file", path: f.path })}><span className="mono">{f.path}</span><span className="faint" style={{ marginLeft: "auto" }}>{bytes(f.size)}</span></button>)}
        {q.startsWith("/") && <button className="menu-item" onClick={() => onPick({ type: "file", path: q.trim() })}>Follow <span className="mono">{q.trim()}</span></button>}
      </div>
    </div>
  );
}

/** Follow logs live: files, service journals and containers, several at once. */
export function LogsView({ s }: { s: SessionState }) {
  const toast = useStore((x) => x.toast);
  const sid = s.info.id;
  const [sources, setSources] = useState<Followed[]>([]);
  const [lines, setLines] = useState<Line[]>([]);
  const [picking, setPicking] = useState(false);
  const [paused, setPaused] = useState(false);
  const [wrap, setWrap] = useState(true);
  const [q, setQ] = useState("");
  const [onlyProblems, setOnlyProblems] = useState(false);
  const counter = useRef(0);
  const pending = useRef<Line[]>([]);
  const partial = useRef<Record<string, string>>({});
  const pausedRef = useRef(paused);
  pausedRef.current = paused;
  const box = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const live = useRef<Followed[]>([]);
  live.current = sources;

  // New lines are added in batches, a few times a second, so busy logs stay smooth.
  useEffect(() => {
    const t = setInterval(() => {
      if (pausedRef.current || pending.current.length === 0) return;
      const add = pending.current;
      pending.current = [];
      setLines((l) => [...l, ...add].slice(-MAX_LINES));
    }, 200);
    return () => clearInterval(t);
  }, []);
  useEffect(() => {
    if (stick.current && box.current) box.current.scrollTop = box.current.scrollHeight;
  }, [lines]);
  // Stop every stream when the tab closes.
  useEffect(() => () => { live.current.forEach((f) => f.id && api.stopLogs(f.id)); }, []);

  async function follow(source: LogSource) {
    setPicking(false);
    const key = keyOf(source);
    if (sources.some((f) => f.key === key)) return;
    const idx = sources.length;
    const f: Followed = { key, label: labelOf(source), source, id: null, color: COLORS[idx % COLORS.length], ended: false };
    setSources((x) => [...x, f]);
    try {
      const id = await api.followLogs(sid, source, 200, (m) => {
        if (m.type === "end") {
          setSources((x) => x.map((y) => (y.key === key ? { ...y, ended: true } : y)));
          return;
        }
        const text = (partial.current[key] ?? "") + m.text;
        const parts = text.split("\n");
        partial.current[key] = parts.pop() ?? "";
        for (const p of parts) pending.current.push({ n: ++counter.current, src: idx, text: p.replace(/\r$/, "") });
        if (pending.current.length > MAX_LINES) pending.current = pending.current.slice(-MAX_LINES);
      });
      setSources((x) => x.map((y) => (y.key === key ? { ...y, id } : y)));
    } catch (e) {
      toast({ kind: "error", title: `Couldn't follow ${f.label}`, body: asApiError(e).message });
      setSources((x) => x.filter((y) => y.key !== key));
    }
  }

  async function stop(f: Followed) {
    if (f.id) await api.stopLogs(f.id).catch(() => {});
    setSources((x) => x.filter((y) => y.key !== f.key));
  }

  const shown = useMemo(() => {
    const needle = q.trim().toLowerCase();
    return lines.filter((l) => (!needle || l.text.toLowerCase().includes(needle)) && (!onlyProblems || level(l.text)));
  }, [lines, q, onlyProblems]);

  const mark = (text: string) => {
    const needle = q.trim();
    if (!needle) return text;
    const i = text.toLowerCase().indexOf(needle.toLowerCase());
    if (i < 0) return text;
    return <>{text.slice(0, i)}<mark>{text.slice(i, i + needle.length)}</mark>{text.slice(i + needle.length)}</>;
  };

  return (
    <div className="view">
      <div className="toolbar">
        {sources.map((f) => (
          <span key={f.key} className="log-chip" style={{ borderColor: f.color }} title={f.ended ? "This log stopped" : "Following"}>
            <i style={{ background: f.ended ? "var(--idle)" : f.color }} />
            <span className="mono">{f.label}</span>
            <button className="icon-btn" style={{ width: 18, height: 18 }} aria-label={`Stop following ${f.label}`} onClick={() => stop(f)}><X size={11} /></button>
          </span>
        ))}
        <div className="menu-anchor">
          <button className={`btn ${sources.length === 0 ? "primary" : ""}`} onClick={() => setPicking(!picking)}><Plus size={13} /> {sources.length === 0 ? "Choose a log" : "Add"}</button>
          {picking && <div className="menu log-menu"><SourcePicker s={s} onPick={follow} onClose={() => setPicking(false)} /></div>}
        </div>
        <span className="spacer" />
        <div className="search"><Search size={14} /><input className="input" placeholder="Search" value={q} onChange={(e) => setQ(e.target.value)} /></div>
        <label className="check" style={{ alignItems: "center" }}><input type="checkbox" checked={onlyProblems} onChange={(e) => setOnlyProblems(e.target.checked)} /><span>Errors and warnings only</span></label>
        <button className="icon-btn" aria-label={paused ? "Resume" : "Pause"} title={paused ? "Resume (new lines were kept)" : "Pause, to read"} onClick={() => setPaused(!paused)}>{paused ? <Play size={14} /> : <Pause size={14} />}</button>
        <button className={`icon-btn ${wrap ? "on" : ""}`} aria-label="Wrap long lines" title="Wrap long lines" onClick={() => setWrap(!wrap)}><WrapText size={14} /></button>
        <button className="icon-btn" aria-label="Clear" title="Clear" onClick={() => { setLines([]); pending.current = []; }}><Eraser size={14} /></button>
      </div>
      {sources.length === 0 ? (
        <div className="empty"><div>
          <h3>Watch logs as they are written</h3>
          <p>Follow a log file, a service's journal or a container's output, or several at once, each in its own colour. Search, and show only errors and warnings.</p>
          <div className="chips" style={{ justifyContent: "center" }}>
            {[["/var/log/syslog", "System log"], ["/var/log/auth.log", "Logins (auth.log)"], ["/var/log/nginx/error.log", "nginx errors"], ["/var/log/nginx/access.log", "nginx requests"]].map(([p, l]) => (
              <button key={p} className="chip" onClick={() => follow({ type: "file", path: p })}>{l}</button>
            ))}
          </div>
        </div></div>
      ) : (
        <div className={`log-out ${wrap ? "wrap" : ""}`} ref={box} onScroll={(e) => { const el = e.currentTarget; stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40; }}>
          {shown.map((l) => {
            const lv = level(l.text);
            const src = sources[l.src];
            return (
              <div key={l.n} className={`log-line ${lv ?? ""}`}>
                {sources.length > 1 && <span className="log-src" style={{ color: src?.color }}>{src?.label ?? ""}</span>}
                <span>{mark(l.text)}</span>
              </div>
            );
          })}
          {paused && <div className="log-paused">Paused: new lines are kept and appear when you resume.</div>}
        </div>
      )}
    </div>
  );
}
