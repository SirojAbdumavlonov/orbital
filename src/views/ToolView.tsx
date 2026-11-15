import { FileText, Folder, Play, RefreshCw, RotateCw, Square, Zap } from "lucide-react";
import { useEffect, useState } from "react";
import { confirm, StateDot } from "../components/ui";
import { api, asApiError, ServiceAction, ServiceDetail } from "../lib/api";
import { bytes } from "../lib/format";
import { SessionState, useStore } from "../store";

/** A tab for any detected software without a dedicated view: its service, configuration and logs. */
export function ToolView({ s, toolId }: { s: SessionState; toolId: string }) {
  const { toast, openFile, showFolder, detectTools } = useStore();
  const sid = s.info.id;
  const tool = s.tools?.find((t) => t.id === toolId);
  const unit = tool?.unit ?? null;
  const [detail, setDetail] = useState<ServiceDetail | null>(null);
  const [busy, setBusy] = useState<ServiceAction | null>(null);
  const [loading, setLoading] = useState(false);

  const load = async () => {
    if (!unit) return;
    setLoading(true);
    try {
      setDetail(await api.serviceDetail(sid, unit));
    } catch (e) {
      toast({ kind: "error", title: `Couldn't read ${unit}`, body: asApiError(e).message });
    } finally {
      setLoading(false);
    }
  };
  useEffect(() => { setDetail(null); load(); }, [sid, unit]);

  if (!tool) {
    return <div className="view"><div className="empty"><p>{s.tools === null ? "Looking at what is installed on this server…" : "This software is no longer found on the server."}</p></div></div>;
  }

  const name = unit ? unit.replace(".service", "") : tool.name;
  async function act(action: ServiceAction) {
    if (!unit) return;
    const isSsh = name === "ssh" || name === "sshd";
    const risky: Partial<Record<ServiceAction, string>> = {
      stop: isSsh ? "Stopping SSH will cut off this connection and lock you out of the server." : `Stopping ${name} takes it offline until you start it again.`,
      restart: `${name} will be briefly unavailable while it restarts.`,
    };
    if (risky[action]) {
      const label = `${action[0].toUpperCase() + action.slice(1)} ${name}`;
      const ok = await confirm({ title: `${label}?`, body: risky[action], confirmLabel: label, danger: action !== "restart", requireText: isSsh && action === "stop" ? name : undefined });
      if (!ok) return;
    }
    setBusy(action);
    try {
      const rep = await api.serviceAction(sid, unit, action);
      if (rep.output.exitCode === 0) toast({ kind: "success", title: `${name}: ${action} done`, body: rep.preflight ? `${rep.preflight[0]} passed` : undefined });
      else toast({ kind: "error", title: `${name}: ${action} failed`, body: rep.output.stderr || rep.output.stdout });
    } catch (e) {
      const err = asApiError(e);
      if (err.kind === "PreflightFailed") toast({ kind: "error", title: `Not ${action}ed: ${err.data.check} found errors`, body: err.data.output });
      else toast({ kind: "error", title: `${name}: ${action} failed`, body: err.message });
    } finally {
      setBusy(null);
      await detectTools(sid);
      load();
    }
  }

  const p = detail?.properties ?? {};
  const active = tool.unitState === "active";
  return (
    <div className="view">
      <div className="toolbar">
        <strong>{tool.name}</strong>
        <span className="faint">{tool.category}</span>
        <StateDot state={tool.running ? "running" : tool.unitState === "failed" ? "failed" : "inactive"} label={tool.running ? "running" : tool.unitState ?? "stopped"} />
        <span className="spacer" />
        {unit && !active && <button className="btn primary" disabled={!!busy} onClick={() => act("start")}><Play size={13} /> Start</button>}
        {unit && active && <button className="btn" disabled={!!busy} onClick={() => act("reload")} title="Reload configuration without dropping connections"><Zap size={13} /> Reload</button>}
        {unit && <button className="btn" disabled={!!busy} onClick={() => act("restart")}><RotateCw size={13} className={busy === "restart" ? "spin" : ""} /> Restart</button>}
        {unit && active && <button className="btn danger" disabled={!!busy} onClick={() => act("stop")}><Square size={13} /> Stop</button>}
        <button className="icon-btn" aria-label="Refresh" onClick={() => { detectTools(sid); load(); }}><RefreshCw size={14} className={loading ? "spin" : ""} /></button>
      </div>
      <div className="split">
        <div className="scroll">
          <div className="section-title">Configuration and logs</div>
          <table className="table">
            <tbody>
              {tool.paths.map((x) => (
                <tr key={x.path} className="click" onClick={() => (x.isDir ? showFolder(sid, x.path) : openFile(sid, x.path))} title={x.isDir ? "Show in Files" : "Open in the editor"}>
                  <td style={{ width: 24 }}>{x.isDir ? <Folder size={14} color="var(--amber)" /> : <FileText size={14} className="faint" />}</td>
                  <td className="mono">{x.path}</td>
                  <td className="faint num">{x.isDir ? "Folder" : "File"}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {tool.paths.length === 0 && <div className="empty" style={{ flex: "none" }}><p>No configuration files found in the usual places.</p></div>}
          <div className="section-title">Details</div>
          <dl className="kv">
            <dt>Program</dt><dd className="mono">{tool.binary ?? "—"}</dd>
            <dt>Service</dt><dd className="mono">{unit ?? "Not managed by systemd"}</dd>
            {unit && <>
              <dt>Main PID</dt><dd>{p.MainPID && p.MainPID !== "0" ? p.MainPID : "—"}</dd>
              <dt>Running since</dt><dd>{p.ActiveEnterTimestamp || "—"}</dd>
              <dt>Memory</dt><dd>{p.MemoryCurrent && /^\d+$/.test(p.MemoryCurrent) ? bytes(Number(p.MemoryCurrent)) : "—"}</dd>
              <dt>Starts at boot</dt><dd>{p.UnitFileState || "—"}</dd>
              <dt>Runs as</dt><dd>{p.User || "root"}</dd>
            </>}
          </dl>
        </div>
        <aside className="drawer">
          <div className="drawer-head"><h3>Recent log</h3><div className="muted">{unit ? `journalctl -u ${unit}` : "Only available for systemd services"}</div></div>
          <pre className="logs">{unit ? detail?.logs ?? "Loading logs…" : "This program runs without a systemd unit (for example inside a container), so Orbital can't start, stop or read logs for it here."}</pre>
        </aside>
      </div>
    </div>
  );
}
