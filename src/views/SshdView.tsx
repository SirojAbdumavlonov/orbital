import { AlertTriangle, CheckCircle2, FileText, Info, RefreshCw, Search, ShieldAlert, ShieldCheck, XCircle, Zap } from "lucide-react";
import { useMemo, useState } from "react";
import { api, asApiError, SshdCheck } from "../lib/api";
import { useRemote } from "../lib/remote";
import { SessionState, useStore } from "../store";

const ICON = { bad: XCircle, warn: AlertTriangle, ok: CheckCircle2, info: Info };
const COLOR = { bad: "var(--bad)", warn: "var(--warn)", ok: "var(--ok)", info: "var(--ink-3)" };

/** The SSH server's real settings, checked against hardening advice, and who is logged in. */
export function SshdView({ s }: { s: SessionState }) {
  const { toast, openFile } = useStore();
  const sid = s.info.id;
  const unit = s.tools?.find((t) => t.id === "sshd")?.unit ?? null;
  const { data: r, error, loading, reload } = useRemote(() => api.sshdReport(sid), [sid]);
  const [q, setQ] = useState("");
  const [reloading, setReloading] = useState(false);

  const settings = useMemo(() => Object.entries(r?.settings ?? {}).filter(([k, v]) => !q || `${k} ${v}`.toLowerCase().includes(q.toLowerCase())), [r, q]);
  const counts = (r?.checks ?? []).reduce((m, c) => ({ ...m, [c.level]: (m[c.level] ?? 0) + 1 }), {} as Record<string, number>);
  const mainFile = r?.configFiles[0] ?? "/etc/ssh/sshd_config";

  async function reloadSshd() {
    if (!unit) return;
    setReloading(true);
    try {
      const rep = await api.serviceAction(sid, unit, "reload");
      if (rep.output.exitCode === 0) toast({ kind: "success", title: "SSH server reloaded", body: "New logins use the saved settings. This connection stays open." });
      else toast({ kind: "error", title: "Reload failed", body: rep.output.stderr || rep.output.stdout });
    } catch (e) {
      const err = asApiError(e);
      toast({ kind: "error", title: err.kind === "PreflightFailed" ? "Not reloaded: sshd -t found errors" : "Reload failed", body: err.data?.output ?? err.message });
    } finally {
      setReloading(false);
      reload();
    }
  }

  const note = (c: SshdCheck) =>
    c.id === "password-login" && c.value === "yes" && s.profile.authMethod === "password"
      ? "You connect to this server with a password. Add your key to ~/.ssh/authorized_keys and switch this profile to key login before turning passwords off."
      : null;

  return (
    <div className="view">
      <div className="toolbar">
        {r && (r.testOk ? <span className="badge ok"><ShieldCheck size={12} /> Configuration is valid</span> : <span className="badge bad"><ShieldAlert size={12} /> sshd -t found errors</span>)}
        {r?.version && <span className="faint">{r.version}</span>}
        {r?.readable && <span className="faint">{counts.bad ?? 0} problems · {counts.warn ?? 0} suggestions</span>}
        <span className="spacer" />
        <button className="btn" onClick={() => openFile(sid, mainFile)}><FileText size={13} /> Edit sshd_config</button>
        {unit && <button className="btn" disabled={reloading} title="Apply saved changes. sshd -t runs first, and open connections stay up." onClick={reloadSshd}><Zap size={13} className={reloading ? "spin" : ""} /> Reload SSH</button>}
        <button className="icon-btn" aria-label="Refresh" onClick={reload}><RefreshCw size={14} className={loading ? "spin" : ""} /></button>
      </div>
      {error && <div className="empty"><div><h3>Couldn't read the SSH server settings</h3><p>{error.message}</p><button className="btn" onClick={reload}>Try again</button></div></div>}
      {r && !r.testOk && r.testOutput && <div className="alert" style={{ margin: 12 }}><b>sshd -t:</b><pre className="output" style={{ marginTop: 6 }}>{r.testOutput}</pre></div>}
      {r && !r.readable && <div className="notice">Reading the settings sshd uses (<span className="mono">sshd -T</span>) needs root. Turn on sudo in this server's settings, then reconnect, to see the security review.</div>}
      {r && (
        <div className="split">
          <div className="scroll">
            <div className="checks">
              {r.checks.map((c) => {
                const Icon = ICON[c.level];
                const extra = note(c);
                return (
                  <div key={c.id} className="check-card">
                    <Icon size={16} color={COLOR[c.level]} style={{ flex: "none", marginTop: 2 }} />
                    <div style={{ minWidth: 0 }}>
                      <div className="check-title"><b>{c.title}</b><span className="mono faint">{c.setting} {c.value}</span></div>
                      <div className="muted">{c.advice}</div>
                      {extra && <div className="check-extra">{extra}</div>}
                    </div>
                  </div>
                );
              })}
            </div>
            <div className="section-title">Configuration files, in the order sshd reads them</div>
            <table className="table"><tbody>
              {r.configFiles.map((f) => (
                <tr key={f} className="click" onClick={() => openFile(sid, f)} title="Open in the editor"><td style={{ width: 24 }}><FileText size={14} className="faint" /></td><td className="mono">{f}</td></tr>
              ))}
            </tbody></table>
            <p className="footnote">For most settings the first value sshd reads wins, so a file in sshd_config.d overrides the same line in sshd_config. Saving runs <span className="mono">sshd -t</span> and rolls back a config that wouldn't load; then reload SSH to apply it.</p>
          </div>
          <aside className="drawer">
            <div className="drawer-head"><h3>Logged in now</h3><div className="muted">{r.sessions.length} sessions</div></div>
            <div className="scroll" style={{ flex: "none", maxHeight: 220 }}>
              <table className="table"><tbody>
                {r.sessions.map((x, i) => <tr key={i}><td className="mono">{x.user}</td><td className="mono muted">{x.from ?? x.tty}</td><td className="faint nowrap">{x.since}</td></tr>)}
              </tbody></table>
            </div>
            <div className="section-title">All settings sshd uses</div>
            <div className="search" style={{ padding: "0 12px 8px" }}><Search size={14} /><input className="input" placeholder="Filter" value={q} onChange={(e) => setQ(e.target.value)} /></div>
            <div className="scroll">
              <table className="table"><tbody>
                {settings.map(([k, v]) => <tr key={k}><td className="mono muted">{k}</td><td className="mono"><div className="cell-clip" style={{ maxWidth: 220 }} title={v}>{v}</div></td></tr>)}
              </tbody></table>
            </div>
          </aside>
        </div>
      )}
    </div>
  );
}
