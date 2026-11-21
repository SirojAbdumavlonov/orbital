import { Play, ScrollText, RefreshCw, RotateCw, Search, Square, ToggleLeft, ToggleRight, Zap } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { confirm, StateDot, DrawerClose } from "../components/ui";
import { api, asApiError, Service, ServiceAction, ServiceDetail } from "../lib/api";
import { runJob, useRunningJob } from "../lib/jobs";
import { JobProgress } from "../components/JobProgress";
import { rangeSeconds, useLogPane } from "../components/LogPane";
import { bytes } from "../lib/format";
import { SessionState, useStore } from "../store";
import { useKept } from "../lib/kept";

type Filter = "all" | "running" | "failed" | "stopped";

export function ServicesView({ s }: { s: SessionState }) {
  const toast = useStore((x) => x.toast);
  const sid = s.info.id;
  const [list, setList] = useState<Service[]>([]);
  const [loading, setLoading] = useState(false);
  const [q, setQ] = useKept(`${s.info.id}:services:q`, "");
  const [filter, setFilter] = useKept<Filter>(`${s.info.id}:services:filter`, "running");
  const [sel, setSel] = useKept<string | null>(`${s.info.id}:services:sel`, null);
  const [detail, setDetail] = useState<ServiceDetail | null>(null);
  const [busy, setBusy] = useState<ServiceAction | null>(null);
  const job = useRunningJob(`service:${sid}`);
  const log = useLogPane({
    sid,
    id: sel,
    name: (sel ?? "service").replace(/\.service$/, ""),
    follow: sel ? { type: "unit", unit: sel } : null,
    load: (r) => api.unitLogs(sid, sel!, r ? 5000 : 400, r ? rangeSeconds(r.from) : null, r ? rangeSeconds(r.to) : null),
  });

  const load = async () => {
    setLoading(true);
    try {
      setList(await api.listServices(sid));
    } catch (e) {
      toast({ kind: "error", title: "Couldn't list services", body: asApiError(e).message });
    } finally {
      setLoading(false);
    }
  };
  const loadDetail = async (unit: string) => {
    try {
      setDetail(await api.serviceDetail(sid, unit));
    } catch (e) {
      toast({ kind: "error", title: `Couldn't read ${unit}`, body: asApiError(e).message });
    }
  };
  useEffect(() => { load(); }, [sid]);
  useEffect(() => { setDetail(null); if (sel) loadDetail(sel); }, [sel]);

  const counts = useMemo(() => ({
    all: list.length,
    running: list.filter((x) => x.active === "active").length,
    failed: list.filter((x) => x.active === "failed").length,
    stopped: list.filter((x) => x.active === "inactive").length,
  }), [list]);
  const shown = list.filter((x) =>
    (filter === "all" || (filter === "running" ? x.active === "active" : filter === "failed" ? x.active === "failed" : x.active === "inactive")) &&
    (!q || x.unit.toLowerCase().includes(q.toLowerCase()) || x.description.toLowerCase().includes(q.toLowerCase())));
  const cur = list.find((x) => x.unit === sel);

  async function act(action: ServiceAction) {
    if (!cur) return;
    const name = cur.unit.replace(".service", "");
    const risky: Partial<Record<ServiceAction, string>> = {
      stop: `Stopping ${name} takes it offline until you start it again.`,
      restart: `${name} will be briefly unavailable while it restarts.`,
      disable: `${name} won't start automatically after the next reboot.`,
    };
    if (name === "ssh" || name === "sshd") risky.stop = "Stopping SSH will cut off this connection and lock you out of the server.";
    if (risky[action]) {
      const ok = await confirm({
        title: `${action[0].toUpperCase() + action.slice(1)} ${name}?`,
        body: risky[action],
        confirmLabel: `${action[0].toUpperCase() + action.slice(1)} ${name}`,
        danger: action !== "restart",
        requireText: (name === "ssh" || name === "sshd") && action === "stop" ? name : undefined,
      });
      if (!ok) return;
    }
    setBusy(action);
    try {
      await runJob((p) => api.serviceAction(sid, cur.unit, action, p), {
        key: `service:${sid}`,
        title: `${name}: ${action}`,
        sid,
        quietHere: false,
        done: (r) => (r.output.exitCode === 0 ? `${name}: ${action} done${r.preflight ? ` (${r.preflight[0]} passed)` : ""}` : { failed: (r.output.stderr || r.output.stdout).trim() || `exit code ${r.output.exitCode}` }),
      });
    } catch (e) {
      const err = asApiError(e);
      // Other errors are reported by the job; a failed check shows what it found.
      if (err.kind === "PreflightFailed") toast({ kind: "error", title: `Not ${action}ed: ${err.data.check} found errors`, body: err.data.output });
    } finally {
      setBusy(null);
      load();
      loadDetail(cur.unit);
    }
  }

  const p = detail?.properties ?? {};
  return (
    <div className="view">
      <div className="toolbar">
        <div className="segmented" role="tablist">
          {(["running", "failed", "stopped", "all"] as Filter[]).map((f) => (
            <button key={f} className={filter === f ? "on" : ""} onClick={() => setFilter(f)}>
              {f[0].toUpperCase() + f.slice(1)} <span className="faint">{counts[f]}</span>
            </button>
          ))}
        </div>
        <div className="search"><Search size={14} /><input className="input" placeholder="Find a service" value={q} onChange={(e) => setQ(e.target.value)} /></div>
        <span className="spacer" />
        <button className="icon-btn" aria-label="Refresh" onClick={load}><RefreshCw size={14} className={loading ? "spin" : ""} /></button>
      </div>
      <div className="split">
        <div className="scroll">
          <table className="table">
            <thead><tr><th>Service</th><th>State</th><th>Boot</th><th>Description</th></tr></thead>
            <tbody>
              {shown.map((x) => (
                <tr key={x.unit} className={`click ${sel === x.unit ? "sel" : ""}`} onClick={() => setSel(x.unit)}>
                  <td className="mono">{x.unit.replace(".service", "")}</td>
                  <td><StateDot state={x.active === "active" ? "running" : x.active} label={x.sub} /></td>
                  <td className="faint">{x.unitFileState ?? "—"}</td>
                  <td className="muted" style={{ maxWidth: 360, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{x.description}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {!loading && shown.length === 0 && <div className="empty"><p>No {filter === "all" ? "" : filter} services{q ? " match your search" : ""}.</p></div>}
        </div>
        <aside className="drawer">
          {!cur ? (
            <div className="empty"><p>Select a service to see its status, logs and controls.</p></div>
          ) : (
            <>
              <div className="drawer-head">
                <DrawerClose onClose={() => setSel(null)} />
                <h3 className="mono">{cur.unit}</h3>
                <div className="muted">{cur.description}</div>
              </div>
              <div className="actions">
                {cur.active !== "active" && <button className="btn primary" disabled={!!busy || !!job} onClick={() => act("start")}><Play size={13} /> Start</button>}
                {cur.active === "active" && <button className="btn" disabled={!!busy || !!job} onClick={() => act("reload")} title="Reload configuration without dropping connections"><Zap size={13} /> Reload</button>}
                <button className="btn" disabled={!!busy || !!job} onClick={() => act("restart")}><RotateCw size={13} className={busy === "restart" ? "spin" : ""} /> Restart</button>
                {cur.active === "active" && <button className="btn danger" disabled={!!busy || !!job} onClick={() => act("stop")}><Square size={13} /> Stop</button>}
                {cur.unitFileState === "enabled"
                  ? <button className="btn ghost" disabled={!!busy || !!job} onClick={() => act("disable")}><ToggleRight size={14} /> Don't start at boot</button>
                  : cur.unitFileState === "disabled" && <button className="btn ghost" disabled={!!busy || !!job} onClick={() => act("enable")}><ToggleLeft size={14} /> Start at boot</button>}
              </div>
              {job && <JobProgress job={job} compact />}
              <dl className="kv">
                <dt>Main PID</dt><dd>{p.MainPID && p.MainPID !== "0" ? p.MainPID : "—"}</dd>
                <dt>Running since</dt><dd>{p.ActiveEnterTimestamp || "—"}</dd>
                <dt>Memory</dt><dd>{p.MemoryCurrent && /^\d+$/.test(p.MemoryCurrent) ? bytes(Number(p.MemoryCurrent)) : "—"}</dd>
                <dt>Tasks</dt><dd>{p.TasksCurrent && /^\d+$/.test(p.TasksCurrent) ? p.TasksCurrent : "—"}</dd>
                <dt>Restarts</dt><dd>{p.NRestarts ?? "—"}</dd>
                <dt>Runs as</dt><dd>{p.User || "root"}</dd>
                <dt>Unit file</dt><dd className="mono">{p.FragmentPath || "—"}</dd>
              </dl>
              <div className="vsplit-head"><span className="vsplit-title" style={{ paddingLeft: 6 }}><ScrollText size={13} /> Logs</span><span className="spacer" />{log.extra}</div>
              {log.body}
            </>
          )}
        </aside>
      </div>
    </div>
  );
}
