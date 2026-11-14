import { Braces, Pause, Play, RefreshCw, RotateCw, ScrollText, Search, Square, Trash2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { confirm, StateDot } from "../components/ui";
import { api, asApiError, Container, ContainerAction, ContainerStats } from "../lib/api";
import { SessionState, useStore } from "../store";

export function DockerView({ s }: { s: SessionState }) {
  const toast = useStore((x) => x.toast);
  const sid = s.info.id;
  const [list, setList] = useState<Container[]>([]);
  const [stats, setStats] = useState<ContainerStats[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [q, setQ] = useState("");
  const [sel, setSel] = useState<string | null>(null);
  const [pane, setPane] = useState<"logs" | "inspect">("logs");
  const [logs, setLogs] = useState("");
  const [inspect, setInspect] = useState<any>(null);
  const [busy, setBusy] = useState(false);

  const load = async () => {
    if (!s.info.dockerVersion) return;
    setLoading(true);
    try {
      setList(await api.listContainers(sid));
      setError(null);
      api.containerStats(sid).then(setStats).catch(() => {});
    } catch (e) {
      setError(asApiError(e).message);
    } finally {
      setLoading(false);
    }
  };
  useEffect(() => { load(); const t = setInterval(load, 15000); return () => clearInterval(t); }, [sid]);
  const cur = list.find((c) => c.id === sel);
  useEffect(() => {
    if (!cur) return;
    setLogs("Loading…");
    setInspect(null);
    api.containerLogs(sid, cur.id, 400).then(setLogs).catch((e) => setLogs(asApiError(e).message));
    api.inspectContainer(sid, cur.id).then(setInspect).catch(() => {});
  }, [sel]);

  const groups = useMemo(() => {
    const g = new Map<string, Container[]>();
    for (const c of list.filter((c) => !q || `${c.names} ${c.image}`.toLowerCase().includes(q.toLowerCase()))) {
      const k = c.composeProject ?? "";
      g.set(k, [...(g.get(k) ?? []), c]);
    }
    return [...g.entries()].sort(([a], [b]) => (a === "" ? 1 : b === "" ? -1 : a.localeCompare(b)));
  }, [list, q]);
  const statOf = (c: Container) => stats.find((x) => c.id.startsWith(x.id) || x.name === c.names);

  async function act(action: ContainerAction) {
    if (!cur) return;
    const destructive = action === "remove" || action === "forceRemove" || action === "stop";
    if (destructive) {
      const ok = await confirm({
        title: action === "stop" ? `Stop ${cur.names}?` : `Remove ${cur.names}?`,
        body: action === "stop"
          ? "The container stops serving until you start it again."
          : <>The container is deleted. Data in named volumes is kept; anything written only inside the container is lost.{action === "forceRemove" && " It will be killed first if running."}</>,
        confirmLabel: action === "stop" ? "Stop container" : "Remove container",
        danger: true,
        requireText: action === "stop" ? undefined : cur.names,
      });
      if (!ok) return;
    }
    setBusy(true);
    try {
      await api.containerAction(sid, cur.id, action);
      toast({ kind: "success", title: `${cur.names}: ${action === "forceRemove" ? "removed" : action} done` });
      if (action === "remove" || action === "forceRemove") setSel(null);
    } catch (e) {
      toast({ kind: "error", title: `${cur.names}: ${action} failed`, body: asApiError(e).message });
    } finally {
      setBusy(false);
      load();
    }
  }

  if (!s.info.dockerVersion) {
    return <div className="empty"><div><h3>Docker isn't available on this server</h3><p>Outpost looks for the <span className="mono">docker</span> command. Install Docker Engine, or add {s.profile.username} to the <span className="mono">docker</span> group, then reconnect.</p></div></div>;
  }

  return (
    <div className="view">
      <div className="toolbar">
        <div className="search"><Search size={14} /><input className="input" placeholder="Find a container or image" value={q} onChange={(e) => setQ(e.target.value)} /></div>
        <span className="faint">Docker {s.info.dockerVersion} · {list.filter((c) => c.state === "running").length} of {list.length} running</span>
        <span className="spacer" />
        <button className="icon-btn" aria-label="Refresh" onClick={load}><RefreshCw size={14} className={loading ? "spin" : ""} /></button>
      </div>
      {error && <div className="alert" style={{ margin: 12 }}>{error}</div>}
      <div className="split">
        <div className="scroll">
          <table className="table">
            <thead><tr><th>Container</th><th>State</th><th>Image</th><th className="num">CPU</th><th className="num">Memory</th><th>Ports</th></tr></thead>
            {groups.map(([proj, cs]) => (
              <tbody key={proj || "_"}>
                {proj && <tr><td colSpan={6} className="faint" style={{ paddingTop: 14, fontSize: 12, fontWeight: 500 }}>Compose project {proj}</td></tr>}
                {cs.map((c) => {
                  const st = statOf(c);
                  return (
                    <tr key={c.id} className={`click ${sel === c.id ? "sel" : ""}`} onClick={() => setSel(c.id)}>
                      <td className="mono nowrap">{c.names}</td>
                      <td className="nowrap"><StateDot state={c.state} label={c.status} nonzero={c.state === "exited" && !c.status.includes("(0)")} /></td>
                      <td className="muted mono" style={{ maxWidth: 220, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{c.image}</td>
                      <td className="num">{st?.cpuPercent ?? "—"}</td>
                      <td className="num">{st ? st.memUsage.split(" / ")[0] : "—"}</td>
                      <td className="faint mono" style={{ fontSize: 11.5, maxWidth: 200, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{c.ports || "—"}</td>
                    </tr>
                  );
                })}
              </tbody>
            ))}
          </table>
          {!loading && list.length === 0 && !error && <div className="empty"><p>No containers on this server yet.</p></div>}
        </div>
        <aside className="drawer">
          {!cur ? (
            <div className="empty"><p>Select a container to see its logs and controls.</p></div>
          ) : (
            <>
              <div className="drawer-head">
                <h3 className="mono">{cur.names}</h3>
                <div className="muted mono" style={{ fontSize: 12 }}>{cur.id.slice(0, 12)} · {cur.image}</div>
              </div>
              <div className="actions">
                {cur.state !== "running" && cur.state !== "paused" && <button className="btn primary" disabled={busy} onClick={() => act("start")}><Play size={13} /> Start</button>}
                {cur.state === "running" && <button className="btn" disabled={busy} onClick={() => act("restart")}><RotateCw size={13} /> Restart</button>}
                {cur.state === "running" && <button className="btn" disabled={busy} onClick={() => act("pause")}><Pause size={13} /> Pause</button>}
                {cur.state === "paused" && <button className="btn" disabled={busy} onClick={() => act("unpause")}><Play size={13} /> Resume</button>}
                {cur.state === "running" && <button className="btn danger" disabled={busy} onClick={() => act("stop")}><Square size={13} /> Stop</button>}
                <button className="btn danger" disabled={busy} onClick={() => act(cur.state === "running" ? "forceRemove" : "remove")}><Trash2 size={13} /> Remove</button>
              </div>
              <div className="toolbar">
                <div className="segmented">
                  <button className={pane === "logs" ? "on" : ""} onClick={() => setPane("logs")}><ScrollText size={12} /> Logs</button>
                  <button className={pane === "inspect" ? "on" : ""} onClick={() => setPane("inspect")}><Braces size={12} /> Inspect</button>
                </div>
                <span className="spacer" />
                {pane === "logs" && <button className="icon-btn" aria-label="Refresh logs" onClick={() => api.containerLogs(sid, cur.id, 400).then(setLogs)}><RefreshCw size={13} /></button>}
              </div>
              <pre className="logs">{pane === "logs" ? logs : inspect ? JSON.stringify(inspect, null, 2) : "Loading…"}</pre>
            </>
          )}
        </aside>
      </div>
    </div>
  );
}
