import { Ban, FileText, FolderOpen, Play, RefreshCw, ShieldOff } from "lucide-react";
import { useState } from "react";
import { confirm, Modal } from "../components/ui";
import { api, asApiError, Fail2banStatus } from "../lib/api";
import { seconds, useRemote } from "../lib/remote";
import { SessionState, useStore } from "../store";
import { useKept } from "../lib/kept";

/** Fail2ban jails: who is banned, unban or ban an address, and where each jail looks. */
export function Fail2banView({ s }: { s: SessionState }) {
  const { toast, openFile, showFolder, detectTools } = useStore();
  const sid = s.info.id;
  const tool = s.tools?.find((t) => t.id === "fail2ban");
  const { data: st, setData, error, loading, reload } = useRemote(() => api.fail2banStatus(sid), [sid]);
  const [sel, setSel] = useKept<string | null>(`${s.info.id}:fail2ban:sel`, null);
  const [banIp, setBanIp] = useState("");
  const [unbanAll, setUnbanAll] = useState(false);
  const [anyIp, setAnyIp] = useState("");
  const [busy, setBusy] = useState(false);

  const jail = st?.jails.find((j) => j.name === sel) ?? st?.jails[0] ?? null;
  const jailLocal = tool?.paths.find((p) => p.path === "/etc/fail2ban/jail.local");

  async function run(what: string, fn: () => Promise<Fail2banStatus>, done: string) {
    setBusy(true);
    try {
      setData(await fn());
      toast({ kind: "success", title: done });
      return true;
    } catch (e) {
      // Banning this connection's own address is refused outright; the message says why.
      const err = asApiError(e);
      toast({ kind: "error", title: `Couldn't ${what}`, body: err.data?.output ?? err.message });
      return false;
    } finally {
      setBusy(false);
    }
  }

  const unban = (ip: string, j: string | null) => run("unban", () => api.fail2banUnban(sid, j, ip), j ? `${ip} unbanned from ${j}` : `${ip} unbanned everywhere`);

  async function ban() {
    if (!jail) return;
    const ip = banIp.trim();
    const ok = await confirm({ title: `Ban ${ip}?`, body: `${ip} won't be able to reach the services ${jail.name} protects for ${seconds(jail.bantime)}.`, confirmLabel: "Ban", danger: true });
    if (ok && (await run("ban", () => api.fail2banBan(sid, jail.name, ip), `${ip} banned in ${jail.name}`))) setBanIp("");
  }

  async function start() {
    if (!tool?.unit) return;
    setBusy(true);
    try {
      await api.serviceAction(sid, tool.unit, "start");
      toast({ kind: "success", title: "fail2ban started" });
    } catch (e) {
      toast({ kind: "error", title: "Couldn't start fail2ban", body: asApiError(e).message });
    } finally {
      setBusy(false);
      detectTools(sid);
      reload();
    }
  }

  const banned = st?.jails.reduce((n, j) => n + j.currentlyBanned, 0) ?? 0;
  return (
    <div className="view">
      <div className="toolbar">
        {st && (st.running ? <span className="badge ok">Running</span> : <span className="badge bad">Not running</span>)}
        {st?.version && <span className="faint">fail2ban {st.version}</span>}
        {st?.running && <span className="faint">{st.jails.length} jails · {banned} addresses banned now</span>}
        <span className="spacer" />
        {jailLocal
          ? <button className="btn" onClick={() => openFile(sid, jailLocal.path)}><FileText size={13} /> Edit jail.local</button>
          : <button className="btn" onClick={() => showFolder(sid, "/etc/fail2ban")}><FolderOpen size={13} /> Open /etc/fail2ban</button>}
        {st?.running && <button className="btn" disabled={busy} onClick={() => setUnbanAll(true)}><ShieldOff size={13} /> Unban an address</button>}
        <button className="icon-btn" aria-label="Refresh" onClick={reload}><RefreshCw size={14} className={loading ? "spin" : ""} /></button>
      </div>
      {error && <div className="empty"><div><h3>Couldn't read fail2ban</h3><p>{error.message}</p><button className="btn" onClick={reload}>Try again</button></div></div>}
      {st && !st.running && (
        <div className="empty"><div>
          <h3>fail2ban isn't running</h3>
          <p>Nothing is being banned. {st.error}</p>
          {tool?.unit && <button className="btn primary" disabled={busy} onClick={start}><Play size={13} /> Start fail2ban</button>}
        </div></div>
      )}
      {st?.running && (
        <div className="split">
          <div className="scroll">
            <table className="table">
              <thead><tr><th>Jail</th><th className="num">Banned now</th><th className="num">Bans in total</th><th className="num">Failures now</th><th>Watching</th></tr></thead>
              <tbody>
                {st.jails.map((j) => (
                  <tr key={j.name} className={`click ${jail?.name === j.name ? "sel" : ""}`} onClick={() => setSel(j.name)}>
                    <td className="mono"><b>{j.name}</b></td>
                    <td className="num">{j.currentlyBanned > 0 ? <span className="badge bad">{j.currentlyBanned}</span> : 0}</td>
                    <td className="num muted">{j.totalBanned}</td>
                    <td className="num muted">{j.currentlyFailed}</td>
                    <td className="mono faint"><div className="cell-clip" style={{ maxWidth: 260 }} title={j.watching.join(" ")}>{j.watching.join(" ") || "—"}</div></td>
                  </tr>
                ))}
              </tbody>
            </table>
            {st.jails.length === 0 && <div className="empty" style={{ flex: "none" }}><p>No jails are enabled. Enable one (for example <span className="mono">[sshd] enabled = true</span>) in jail.local.</p></div>}
            <p className="footnote">A jail watches a log for failed logins. After <i>max retry</i> failures within <i>find time</i>, the address is banned for <i>ban time</i>.{st.clientIp && <> This connection comes from <span className="mono">{st.clientIp}</span>.</>}</p>
          </div>
          <aside className="drawer">
            {!jail ? <div className="empty"><p>Select a jail.</p></div> : (
              <>
                <div className="drawer-head"><h3 className="mono">{jail.name}</h3><div className="muted">{jail.currentlyBanned} banned now, {jail.totalBanned} since fail2ban started</div></div>
                <dl className="kv">
                  <dt>Bans after</dt><dd>{jail.maxretry ?? "—"} failures within {seconds(jail.findtime)}</dd>
                  <dt>Ban lasts</dt><dd>{seconds(jail.bantime)}</dd>
                  <dt>Never bans</dt><dd className="mono">{jail.ignoreIp.join(", ") || "—"}</dd>
                </dl>
                <div className="section-title">Banned addresses</div>
                <div className="scroll" style={{ flex: "none", maxHeight: 280 }}>
                  <table className="table"><tbody>
                    {jail.banned.map((ip) => (
                      <tr key={ip}>
                        <td className="mono">{ip}{ip === st.clientIp && <span className="badge amber" style={{ marginLeft: 6 }}>you</span>}</td>
                        <td style={{ textAlign: "right" }}><button className="btn" disabled={busy} onClick={() => unban(ip, jail.name)}>Unban</button></td>
                      </tr>
                    ))}
                  </tbody></table>
                  {jail.banned.length === 0 && <div className="empty" style={{ flex: "none", padding: 12 }}><p>Nobody is banned in this jail.</p></div>}
                </div>
                <div className="section-title">Ban an address</div>
                <div className="actions" style={{ paddingTop: 0, flexWrap: "nowrap", borderBottom: 0 }}>
                  <input className="input mono" style={{ flex: 1, width: "auto" }} placeholder="203.0.113.7" value={banIp} onChange={(e) => setBanIp(e.target.value)} onKeyDown={(e) => e.key === "Enter" && banIp.trim() && ban()} />
                  <button className="btn danger" disabled={busy || !banIp.trim()} onClick={ban}><Ban size={13} /> Ban</button>
                </div>
              </>
            )}
          </aside>
        </div>
      )}
      {unbanAll && (
        <Modal title="Unban an address" subtitle="Removes the address from every jail, for example when you locked yourself out from another machine." onClose={() => setUnbanAll(false)}
          footer={<><button className="btn" onClick={() => setUnbanAll(false)}>Cancel</button><button className="btn primary" disabled={busy || !anyIp.trim()} onClick={async () => { if (await unban(anyIp.trim(), null)) { setUnbanAll(false); setAnyIp(""); } }}>Unban everywhere</button></>}>
          <label className="field"><span>IP address</span><input className="input mono" autoFocus placeholder="203.0.113.7" value={anyIp} onChange={(e) => setAnyIp(e.target.value)} /></label>
        </Modal>
      )}
    </div>
  );
}
