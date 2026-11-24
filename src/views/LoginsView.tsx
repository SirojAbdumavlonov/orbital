import { Ban, LogIn, RefreshCw, ShieldAlert, Users } from "lucide-react";
import { useEffect, useState } from "react";
import { confirm } from "../components/ui";
import { api, asApiError, Fail2banStatus, LoginsReport } from "../lib/api";
import { SessionState, useStore } from "../store";

// Logins: who is signed in now, recent sign-ins, and failed SSH attempts over
// the last 24 hours with the busiest addresses. With Fail2ban, an address can
// be banned from here (Fail2ban's own checks apply: your own address is refused).

export function LoginsView({ s }: { s: SessionState }) {
  const sid = s.info.id;
  const [r, setR] = useState<LoginsReport | null>(null);
  const [f2b, setF2b] = useState<Fail2banStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const toast = useStore((x) => x.toast);
  const hasF2b = !!s.tools?.some((t) => t.id === "fail2ban");
  const load = async () => {
    setLoading(true);
    try {
      setR(await api.logins(sid));
      setError(null);
      if (hasF2b) setF2b(await api.fail2banStatus(sid).catch(() => null));
    } catch (e) {
      setError(asApiError(e).message);
    } finally {
      setLoading(false);
    }
  };
  useEffect(() => { void load(); }, [sid]);
  const jail = f2b?.jails.find((j) => j.name === "sshd") ?? f2b?.jails[0];
  const banned = new Set(f2b?.jails.flatMap((j) => j.banned) ?? []);

  const ban = async (ip: string) => {
    if (!jail) return;
    const ok = await confirm({ title: `Ban ${ip}?`, body: <>Fail2ban's <span className="mono">{jail.name}</span> jail blocks it for {jail.bantime ?? "its ban time"}. You can unban it in the Fail2ban tab.</>, confirmLabel: "Ban", danger: true });
    if (!ok) return;
    try {
      setF2b(await api.fail2banBan(sid, jail.name, ip));
      toast({ kind: "success", title: `${ip} is banned`, body: `In the ${jail.name} jail` });
    } catch (e) {
      toast({ kind: "error", title: `Couldn't ban ${ip}`, body: asApiError(e).message });
    }
  };

  return (
    <div className="view">
      <div className="toolbar view-toolbar">
        <span className="faint" style={{ fontSize: 12.5 }}>Who signs in to {s.profile.name}, and who tries to.</span>
        <span className="spacer" />
        <button className="icon-btn" aria-label="Refresh" onClick={() => void load()}><RefreshCw size={14} className={loading ? "spin" : ""} /></button>
      </div>
      <div className="scroll">
        {error && <div className="alert" style={{ margin: 16 }}>{error}</div>}
        {r && (
          <div className="logins">
            <section className="panel">
              <h4><Users size={14} /> Signed in now <span className="faint">{r.now.length}</span></h4>
              {r.now.length === 0 ? <p className="faint">Nobody (Orbital's own connection doesn't count: it has no terminal).</p> : (
                <table className="table"><tbody>{r.now.map((x, i) => <tr key={i}><td className="mono">{x.user}</td><td className="mono faint">{x.tty}</td><td className="mono">{x.from || "console"}</td><td className="faint">{x.when}</td></tr>)}</tbody></table>
              )}
            </section>
            <section className="panel">
              <h4><ShieldAlert size={14} /> Failed SSH attempts, last 24 hours <span className={r.failed > 100 ? "bad-text" : "faint"}>{r.failed}</span></h4>
              {r.source === "" && <p className="faint">The server's SSH log couldn't be read (no journal access or auth.log).</p>}
              {r.failed > 0 && (
                <div className="logins-cols">
                  <div>
                    <div className="faint logins-sub">From</div>
                    <table className="table"><tbody>{r.topAddresses.map((c) => (
                      <tr key={c.key}>
                        <td className="mono">{c.key}</td>
                        <td className="num">{c.count}</td>
                        <td style={{ textAlign: "right" }}>
                          {banned.has(c.key) ? <span className="badge ok">banned</span>
                            : jail ? <button className="btn ghost" onClick={() => void ban(c.key)}><Ban size={12} /> Ban</button> : null}
                        </td>
                      </tr>
                    ))}</tbody></table>
                  </div>
                  <div>
                    <div className="faint logins-sub">Trying the user</div>
                    <table className="table"><tbody>{r.topUsers.map((c) => <tr key={c.key}><td className="mono">{c.key}</td><td className="num">{c.count}</td></tr>)}</tbody></table>
                  </div>
                </div>
              )}
              {r.failed > 0 && !hasF2b && <p className="faint" style={{ fontSize: 12.5 }}>Fail2ban bans addresses that keep failing automatically; install it from the + next to the server's tabs.</p>}
              {r.failed === 0 && r.source !== "" && <p className="faint">None.</p>}
            </section>
            <section className="panel logins-recent">
              <h4><LogIn size={14} /> Recent sign-ins</h4>
              <table className="table"><tbody>{r.recent.map((x, i) => <tr key={i}><td className="mono">{x.user}</td><td className="mono faint">{x.tty}</td><td className="mono">{x.from || "console"}</td><td className="faint">{x.when}</td></tr>)}</tbody></table>
              {r.recent.length === 0 && <p className="faint">No record (wtmp is empty or unreadable).</p>}
            </section>
          </div>
        )}
      </div>
    </div>
  );
}
