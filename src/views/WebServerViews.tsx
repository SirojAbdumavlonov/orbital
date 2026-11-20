import { FileText, Pause, Play, RefreshCw, ShieldAlert, ShieldCheck, Wrench, Zap } from "lucide-react";
import { Fragment, useState } from "react";
import { confirm, StateDot } from "../components/ui";
import { api, asApiError, HaServerState } from "../lib/api";
import { useRemote } from "../lib/remote";
import { SessionState, useStore } from "../store";
import { WithServicePane } from "./DatabaseView";
import { usePlace } from "../components/PlacePicker";

function useReload(s: SessionState, toolId: string, name: string, after: () => void) {
  const toast = useStore((x) => x.toast);
  const unit = s.tools?.find((t) => t.id === toolId)?.unit ?? null;
  const [busy, setBusy] = useState(false);
  const reload = async () => {
    if (!unit) return;
    setBusy(true);
    try {
      const rep = await api.serviceAction(s.info.id, unit, "reload");
      if (rep.output.exitCode === 0) toast({ kind: "success", title: `${name} reloaded`, body: rep.preflight ? `${rep.preflight[0]} passed` : undefined });
      else toast({ kind: "error", title: "Reload failed", body: rep.output.stderr || rep.output.stdout });
    } catch (e) {
      const err = asApiError(e);
      toast({ kind: "error", title: err.kind === "PreflightFailed" ? `Not reloaded: ${err.data.check} found errors` : "Reload failed", body: err.data?.output ?? err.message });
    } finally {
      setBusy(false);
      after();
    }
  };
  return { unit, busy, reload };
}

/** Apache as it is configured: virtual hosts, sites to enable or disable, loaded modules. */
export function ApacheView({ s }: { s: SessionState }) {
  const { toast, openFile } = useStore();
  const sid = s.info.id;
  const [place, picker] = usePlace(s, "apache");
  const { data: st, setData, error, loading, reload } = useRemote(() => api.apacheStatus(sid, place), [sid, place]);
  const svc = useReload(s, "apache", "Apache", reload);
  const [busy, setBusy] = useState<string | null>(null);

  async function toggle(name: string, enable: boolean) {
    if (!enable) {
      const ok = await confirm({ title: `Disable ${name}?`, body: "Its domains stop being served by this site after Apache reloads.", confirmLabel: "Disable site", danger: true });
      if (!ok) return;
    }
    setBusy(name);
    try {
      setData(await api.apacheSite(sid, name, enable));
      toast({ kind: "success", title: `${name} ${enable ? "enabled" : "disabled"}`, body: "Reload Apache to apply it.", action: svc.unit ? { label: "Reload now", run: svc.reload } : undefined });
    } catch (e) {
      const err = asApiError(e);
      toast({ kind: "error", title: err.kind === "PreflightFailed" ? `Not ${enable ? "enabled" : "disabled"}: the configuration would break` : `Couldn't change ${name}`, body: err.data?.output ?? err.message });
    } finally {
      setBusy(null);
    }
  }

  return (
    <WithServicePane s={s} toolId="apache" label="Sites">
      <div className="toolbar">
        {picker}
        {st && (st.testOk ? <span className="badge ok"><ShieldCheck size={12} /> Configuration is valid</span> : <span className="badge bad"><ShieldAlert size={12} /> Configuration has errors</span>)}
        {st && <span className="faint">{st.vhosts.length} virtual hosts · {st.modules.length} modules</span>}
        <span className="spacer" />
        {svc.unit && !place && <button className="btn" disabled={svc.busy} onClick={svc.reload} title="Apply saved changes; apachectl configtest runs first"><Zap size={13} className={svc.busy ? "spin" : ""} /> Reload Apache</button>}
        <button className="icon-btn" aria-label="Refresh" onClick={reload}><RefreshCw size={14} className={loading ? "spin" : ""} /></button>
      </div>
      {error && <div className="empty"><div><h3>Couldn't read Apache</h3><p>{error.message}</p></div></div>}
      {st && !st.testOk && <div className="alert" style={{ margin: 12 }}><pre className="output">{st.testOutput}</pre></div>}
      {st && (
        <div className="scroll">
          <div className="section-title">Virtual hosts Apache serves</div>
          <table className="table">
            <thead><tr><th>Domain names</th><th>Port</th><th>Defined in</th></tr></thead>
            <tbody>
              {st.vhosts.map((v) => (
                <tr key={`${v.file}:${v.line}:${v.port}`} className={place ? "" : "click"} onClick={() => !place && openFile(sid, v.file)} title={place ? `${v.file} inside container ${place}` : "Edit in the editor"}>
                  <td className="mono">{[v.serverName, ...v.aliases].join(", ")}</td>
                  <td className="mono muted">{v.port}</td>
                  <td className="mono faint">{v.file}:{v.line}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {st.vhosts.length === 0 && <div className="empty" style={{ flex: "none" }}><p>No virtual hosts; Apache serves {st.documentRoot ?? "its default folder"} to everyone.</p></div>}
          {st.managesSites && !place && (
            <>
              <div className="section-title">Sites (sites-available)</div>
              <table className="table">
                <tbody>
                  {st.sites.map((x) => (
                    <tr key={x.name}>
                      <td style={{ width: 24 }}><FileText size={14} className="faint" /></td>
                      <td className="mono"><a className="link" onClick={() => openFile(sid, x.file)}>{x.name}</a></td>
                      <td><StateDot state={x.enabled ? "running" : "inactive"} label={x.enabled ? "enabled" : "disabled"} /></td>
                      <td style={{ textAlign: "right" }}>
                        <button className="btn" disabled={busy === x.name} onClick={() => toggle(x.name, !x.enabled)}>{x.enabled ? "Disable" : "Enable"}</button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </>
          )}
          <div className="section-title">Loaded modules</div>
          <div className="chips" style={{ padding: "0 12px 12px" }}>{st.modules.map((m) => <span key={m} className="badge mono">{m}</span>)}</div>
          <p className="footnote">Enabling or disabling a site runs apachectl configtest and undoes the change if it fails. Edits to files go through safe save, which checks them the same way.</p>
        </div>
      )}
    </WithServicePane>
  );
}

const STATUS_STATE: Record<string, string> = { UP: "running", OPEN: "running", DOWN: "failed", MAINT: "inactive", DRAIN: "paused", "no check": "running" };

/** HAProxy: what listens, where traffic goes, and live server health with drain and maintenance. */
export function HaproxyView({ s }: { s: SessionState }) {
  const { toast, openFile } = useStore();
  const sid = s.info.id;
  const { data: st, setData, error, loading, reload } = useRemote(() => api.haproxyStatus(sid), [sid]);
  const svc = useReload(s, "haproxy", "HAProxy", reload);
  const [busy, setBusy] = useState<string | null>(null);

  async function setState(backend: string, server: string, state: HaServerState) {
    if (state !== "ready") {
      const ok = await confirm({
        title: state === "drain" ? `Drain ${server}?` : `Put ${server} in maintenance?`,
        body: state === "drain" ? "It finishes the requests it has and gets no new ones." : "It is taken out of rotation at once. Its open connections are kept until they end.",
        confirmLabel: state === "drain" ? "Drain" : "Maintenance",
      });
      if (!ok) return;
    }
    setBusy(`${backend}/${server}`);
    try {
      setData(await api.haproxyServerState(sid, backend, server, state));
      toast({ kind: "success", title: `${server}: ${state === "ready" ? "back in rotation" : state === "drain" ? "draining" : "in maintenance"}`, body: "Lasts until you change it back or HAProxy restarts." });
    } catch (e) {
      toast({ kind: "error", title: `Couldn't change ${server}`, body: asApiError(e).message });
    } finally {
      setBusy(null);
    }
  }

  const live = !!st?.socket;
  return (
    <WithServicePane s={s} toolId="haproxy" label="Frontends and backends">
      <div className="toolbar">
        {st && (st.testOk ? <span className="badge ok"><ShieldCheck size={12} /> Configuration is valid</span> : <span className="badge bad"><ShieldAlert size={12} /> Configuration has errors</span>)}
        {st && <span className="faint">{live ? "Live status from the stats socket" : "No stats socket: status not available"}</span>}
        <span className="spacer" />
        {st && <button className="btn" onClick={() => openFile(sid, st.configFile)}><FileText size={13} /> Edit haproxy.cfg</button>}
        {svc.unit && <button className="btn" disabled={svc.busy} onClick={svc.reload} title="Apply saved changes; haproxy -c runs first"><Zap size={13} className={svc.busy ? "spin" : ""} /> Reload HAProxy</button>}
        <button className="icon-btn" aria-label="Refresh" onClick={reload}><RefreshCw size={14} className={loading ? "spin" : ""} /></button>
      </div>
      {error && <div className="empty"><div><h3>Couldn't read HAProxy</h3><p>{error.message}</p></div></div>}
      {st && !st.testOk && <div className="alert" style={{ margin: 12 }}><pre className="output">{st.testOutput}</pre></div>}
      {st && (
        <div className="scroll">
          {st.sections.map((sec) => (
            <section key={`${sec.kind}:${sec.name}`} className="ha-section">
              <div className="ha-head">
                <span className="badge">{sec.kind}</span>
                <b className="mono">{sec.name}</b>
                {sec.status && <StateDot state={STATUS_STATE[sec.status] ?? "inactive"} label={sec.status} />}
                <span className="faint mono">{[sec.mode, sec.balance && `balance ${sec.balance}`].filter(Boolean).join(" · ")}</span>
                <span className="spacer" />
                <a className="link faint" onClick={() => openFile(sid, st.configFile)}>line {sec.line}</a>
              </div>
              {(sec.binds.length > 0 || sec.defaultBackend || sec.useBackends.length > 0) && (
                <dl className="kv">
                  {sec.binds.length > 0 && <><dt>Listens on</dt><dd className="mono">{sec.binds.join(", ")}</dd></>}
                  {sec.useBackends.map((u) => <Fragment key={u}><dt>Sends to</dt><dd className="mono">{u}</dd></Fragment>)}
                  {sec.defaultBackend && <><dt>Otherwise to</dt><dd className="mono">{sec.defaultBackend}</dd></>}
                </dl>
              )}
              {sec.servers.length > 0 && (
                <table className="table">
                  <thead><tr><th>Server</th><th>Address</th><th>Status</th><th className="num">Sessions</th><th>Check</th>{live && <th />}</tr></thead>
                  <tbody>
                    {sec.servers.map((sv) => {
                      const key = `${sec.name}/${sv.name}`;
                      return (
                        <tr key={key}>
                          <td className="mono">{sv.name}{sv.options.includes("backup") && <span className="faint"> backup</span>}</td>
                          <td className="mono muted">{sv.address}</td>
                          <td>{sv.status ? <StateDot state={STATUS_STATE[sv.status.split(" ")[0]] ?? "inactive"} label={sv.status} /> : <span className="faint">—</span>}</td>
                          <td className="num">{sv.sessions ?? "—"}</td>
                          <td className="faint mono">{sv.check ?? ""}</td>
                          {live && (
                            <td className="nowrap" style={{ textAlign: "right" }}>
                              {sv.status === "MAINT" || sv.status === "DRAIN"
                                ? <button className="btn" disabled={busy === key} onClick={() => setState(sec.name, sv.name, "ready")}><Play size={13} /> Back in rotation</button>
                                : <>
                                    <button className="btn" disabled={busy === key} onClick={() => setState(sec.name, sv.name, "drain")} title="Finish current requests, take no new ones"><Pause size={13} /> Drain</button>{" "}
                                    <button className="btn" disabled={busy === key} onClick={() => setState(sec.name, sv.name, "maint")} title="Take out of rotation"><Wrench size={13} /> Maintenance</button>
                                  </>}
                            </td>
                          )}
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              )}
            </section>
          ))}
          {st.sections.length === 0 && <div className="empty" style={{ flex: "none" }}><p>No frontends or backends in {st.configFile}.</p></div>}
          {!live && <p className="footnote">To see live server health and drain servers from here, add <span className="mono">stats socket /run/haproxy/admin.sock mode 660 level admin</span> to the global section, and install socat.</p>}
        </div>
      )}
    </WithServicePane>
  );
}
