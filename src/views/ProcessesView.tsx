import { ChevronDown, ChevronRight, ListTree, Pause, Play, RefreshCw, Search, Skull } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { confirm, DrawerClose } from "../components/ui";
import { api, asApiError, Proc, ProcDetail, ProcSignal } from "../lib/api";
import { bytes } from "../lib/format";
import { useKept } from "../lib/kept";
import { SessionState, useStore } from "../store";

type Sort = "cpu" | "mem" | "pid" | "name";
const STATE: Record<string, string> = { R: "running", S: "sleeping", D: "waiting for disk", Z: "zombie (ended, parent didn't collect it)", T: "stopped", I: "idle", t: "traced" };
const SIGNALS: { s: ProcSignal; label: string; hint: string; danger?: boolean }[] = [
  { s: "TERM", label: "Stop", hint: "Ask it to finish and exit (SIGTERM)." },
  { s: "KILL", label: "Kill", hint: "End it at once; it can't clean up (SIGKILL).", danger: true },
  { s: "HUP", label: "Reload", hint: "Many daemons re-read their config on SIGHUP." },
  { s: "INT", label: "Interrupt", hint: "Like Ctrl-C (SIGINT)." },
  { s: "STOP", label: "Pause", hint: "Freeze it until it's resumed (SIGSTOP)." },
  { s: "CONT", label: "Resume", hint: "Continue a paused process (SIGCONT)." },
  { s: "USR1", label: "USR1", hint: "Program-specific (e.g. nginx reopens its logs)." },
  { s: "USR2", label: "USR2", hint: "Program-specific." },
];
const ago = (s: number) => (s < 3600 ? `${Math.floor(s / 60)} min` : s < 86400 ? `${Math.floor(s / 3600)} h` : `${Math.floor(s / 86400)} d`);

/** Every process as a tree or a ranked list, with details, signals and priority. */
export function ProcessesView({ s }: { s: SessionState }) {
  const sid = s.info.id;
  const toast = useStore((x) => x.toast);
  const [list, setList] = useState<Proc[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [q, setQ] = useKept(`${sid}:procs:q`, "");
  const [sort, setSort] = useKept<Sort>(`${sid}:procs:sort`, "cpu");
  const [tree, setTree] = useKept(`${sid}:procs:tree`, false);
  const [live, setLive] = useKept(`${sid}:procs:live`, true);
  const [kthreads, setKthreads] = useKept(`${sid}:procs:kthreads`, false);
  const [sel, setSel] = useKept<number | null>(`${sid}:procs:sel`, null);
  const [closed, setClosed] = useState<Set<number>>(new Set());
  const [detail, setDetail] = useState<ProcDetail | null>(null);
  const [nice, setNice] = useState(0);

  const load = () => api.processes(sid).then((l) => { setList(l); setError(null); }).catch((e) => setError(asApiError(e).message));
  useEffect(() => { void load(); }, [sid]);
  useEffect(() => {
    if (!live) return;
    const t = setInterval(() => void load(), 5000);
    return () => clearInterval(t);
  }, [live, sid]);
  const cur = list?.find((p) => p.pid === sel) ?? null;
  useEffect(() => {
    setDetail(null);
    if (!sel) return;
    api.processDetail(sid, sel).then(setDetail).catch(() => setDetail(null));
  }, [sel]);
  useEffect(() => { if (cur) setNice(cur.nice); }, [cur?.pid]);

  const visible = useMemo(() => {
    const needle = q.trim().toLowerCase();
    return (list ?? []).filter((p) => (kthreads || !p.name.startsWith("[")) && (!needle || `${p.pid} ${p.user} ${p.args}`.toLowerCase().includes(needle)));
  }, [list, q, kthreads]);

  const rows = useMemo(() => {
    const by = (a: Proc, b: Proc) => sort === "cpu" ? b.cpu - a.cpu : sort === "mem" ? b.rssBytes - a.rssBytes : sort === "pid" ? a.pid - b.pid : a.name.localeCompare(b.name);
    if (!tree || q.trim()) return visible.slice().sort(by).map((p) => ({ p, depth: 0, kids: 0 }));
    const ids = new Set(visible.map((p) => p.pid));
    const children = new Map<number, Proc[]>();
    for (const p of visible) {
      const parent = ids.has(p.ppid) ? p.ppid : 0;
      children.set(parent, [...(children.get(parent) ?? []), p]);
    }
    const out: { p: Proc; depth: number; kids: number }[] = [];
    const walk = (pid: number, depth: number) => {
      for (const c of (children.get(pid) ?? []).sort(by)) {
        const kids = children.get(c.pid)?.length ?? 0;
        out.push({ p: c, depth, kids });
        if (!closed.has(c.pid)) walk(c.pid, depth + 1);
      }
    };
    walk(0, 0);
    return out;
  }, [visible, tree, sort, closed, q]);

  async function signal(sig: ProcSignal, label: string, danger?: boolean) {
    if (!cur) return;
    if (sig === "KILL" || sig === "TERM" || sig === "STOP") {
      const ok = await confirm({ title: `${label} ${cur.name} (PID ${cur.pid})?`, body: <span className="mono">{cur.args}</span>, confirmLabel: label, danger: danger || sig !== "STOP" });
      if (!ok) return;
    }
    try {
      await api.signalProcess(sid, cur.pid, sig);
      toast({ kind: "success", title: `Sent ${sig} to ${cur.name}` });
      setTimeout(() => void load(), 400);
    } catch (e) {
      toast({ kind: "error", title: `Couldn't send ${sig}`, body: asApiError(e).message });
    }
  }

  async function renice() {
    if (!cur) return;
    try {
      await api.reniceProcess(sid, cur.pid, nice);
      toast({ kind: "success", title: `${cur.name} now runs at priority ${nice}`, body: nice > 0 ? "Lower priority: it gets CPU after others." : nice < 0 ? "Higher priority: it gets CPU before others." : undefined });
      void load();
    } catch (e) {
      toast({ kind: "error", title: "Couldn't change the priority", body: asApiError(e).message });
    }
  }

  return (
    <div className="view">
      <div className="toolbar view-toolbar">
        <div className="search"><Search size={14} /><input className="input" placeholder="Filter by name, user, PID or command" value={q} onChange={(e) => setQ(e.target.value)} /></div>
        <div className="segmented" aria-label="Sort by">
          {(["cpu", "mem", "pid", "name"] as Sort[]).map((x) => <button key={x} className={sort === x ? "on" : ""} onClick={() => setSort(x)}>{{ cpu: "CPU", mem: "Memory", pid: "PID", name: "Name" }[x]}</button>)}
        </div>
        <button className={`btn ${tree ? "on" : ""}`} title="Show which process started which" onClick={() => setTree(!tree)}><ListTree size={13} /> Tree</button>
        <label className="check" style={{ alignItems: "center" }}><input type="checkbox" checked={kthreads} onChange={(e) => setKthreads(e.target.checked)} /><span>Kernel threads</span></label>
        <span className="spacer" />
        {list && <span className="faint">{visible.length} processes</span>}
        <button className="icon-btn" title={live ? "Pause the 5 s refresh" : "Refresh every 5 s"} aria-label={live ? "Pause refreshing" : "Refresh automatically"} onClick={() => setLive(!live)}>{live ? <Pause size={14} /> : <Play size={14} />}</button>
        <button className="icon-btn" aria-label="Refresh" onClick={() => void load()}><RefreshCw size={14} /></button>
      </div>
      {error && <div className="alert" style={{ margin: 16 }}>{error}</div>}
      <div className={`split ${cur ? "" : "no-drawer"}`}>
        <div className="scroll">
          <table className="table procs-table">
            <thead><tr><th className="num">PID</th><th>Process</th><th>User</th><th className="num">CPU</th><th className="num">Memory</th><th className="num">Running</th></tr></thead>
            <tbody>
              {rows.map(({ p, depth, kids }) => (
                <tr key={p.pid} className={`click ${sel === p.pid ? "sel" : ""} ${p.state.startsWith("Z") ? "faint" : ""}`} onClick={() => setSel(p.pid)}>
                  <td className="num mono faint">{p.pid}</td>
                  <td>
                    <div className="proc-name" style={{ paddingLeft: depth * 16 }}>
                      {tree && !q.trim() && (kids ? <button className="icon-btn proc-fold" aria-label={closed.has(p.pid) ? "Expand" : "Collapse"} onClick={(e) => { e.stopPropagation(); setClosed((c) => { const n = new Set(c); if (n.has(p.pid)) n.delete(p.pid); else n.add(p.pid); return n; }); }}>{closed.has(p.pid) ? <ChevronRight size={12} /> : <ChevronDown size={12} />}</button> : <span className="proc-fold" />)}
                      <b>{p.name}</b><span className="mono faint proc-args" title={p.args}>{p.args.startsWith("[") ? "" : p.args}</span>
                      {p.nice !== 0 && <span className="badge" title="Priority (nice)">nice {p.nice}</span>}
                      {p.state.startsWith("T") && <span className="badge warn">paused</span>}
                    </div>
                  </td>
                  <td className="muted nowrap">{p.user}</td>
                  <td className={`num ${p.cpu >= 50 ? "bad-text" : ""}`}>{p.cpu.toFixed(1)}%</td>
                  <td className="num">{bytes(p.rssBytes)}</td>
                  <td className="num faint nowrap">{ago(p.elapsedSecs)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        {cur && (
          <aside className="drawer">
            <div className="drawer-head">
              <h3 className="mono" style={{ wordBreak: "break-all" }}>{cur.name} <span className="faint">PID {cur.pid}</span></h3>
              <div className="muted">{STATE[cur.state[0]] ?? cur.state} · {cur.user} · started by PID {cur.ppid}</div>
              <DrawerClose onClose={() => setSel(null)} />
            </div>
            <div className="actions">
              {SIGNALS.slice(0, 3).map((x) => <button key={x.s} className={`btn ${x.danger ? "danger" : ""}`} title={x.hint} onClick={() => void signal(x.s, x.label, x.danger)}>{x.s === "KILL" && <Skull size={13} />} {x.label}</button>)}
              <select className="input" style={{ width: 130, height: 28 }} value="" onChange={(e) => { const x = SIGNALS.find((y) => y.s === e.target.value); if (x) void signal(x.s, x.label, x.danger); }} aria-label="Other signals">
                <option value="">Other signal…</option>
                {SIGNALS.slice(3).map((x) => <option key={x.s} value={x.s}>{x.label} ({x.s})</option>)}
              </select>
            </div>
            <dl className="kv">
              <dt>Command</dt><dd className="mono">{cur.args}</dd>
              <dt>CPU · memory</dt><dd>{cur.cpu.toFixed(1)}% · {bytes(cur.rssBytes)} ({cur.mem.toFixed(1)}%)</dd>
              {detail?.threads != null && <><dt>Threads</dt><dd>{detail.threads}</dd></>}
              {detail?.swapBytes ? <><dt>In swap</dt><dd>{bytes(detail.swapBytes)}</dd></> : null}
              {detail?.openFiles != null && <><dt>Open files</dt><dd>{detail.openFiles}{detail.maxFiles ? ` of ${detail.maxFiles}` : ""}</dd></>}
              {detail?.listening.length ? <><dt>Listens on</dt><dd className="mono">{detail.listening.join("\n")}</dd></> : null}
              {detail?.unit && <><dt>Service</dt><dd className="mono">{detail.unit}</dd></>}
              {detail?.cwd && <><dt>Working folder</dt><dd className="mono">{detail.cwd}</dd></>}
              {detail?.exe && <><dt>Program</dt><dd className="mono">{detail.exe}</dd></>}
            </dl>
            <div className="section-title">Priority</div>
            <div className="nice-row">
              <input type="range" min={-20} max={19} value={nice} onChange={(e) => setNice(Number(e.target.value))} aria-label="Nice value" />
              <span className="mono">{nice}</span>
              <button className="btn" disabled={nice === cur.nice} onClick={() => void renice()}>Set</button>
            </div>
            <p className="footnote">-20 runs first, 19 last; 0 is normal. Raising priority (below 0) needs sudo. PID 1 and sshd, which carries this connection, can't be stopped from here.</p>
          </aside>
        )}
      </div>
    </div>
  );
}
