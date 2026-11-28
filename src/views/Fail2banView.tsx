import { Ban, FileText, FolderOpen, Play, RefreshCw, ShieldCheck, ShieldOff } from "lucide-react";
import { useEffect, useState } from "react";
import { confirm, Modal } from "../components/ui";
import { api, asApiError, Fail2banStatus, SshProtection } from "../lib/api";
import { runJob } from "../lib/jobs";
import { seconds, useRemote, withSafetyOverride } from "../lib/remote";
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
  const [protecting, setProtecting] = useState(false);
  // Bans happen on their own: look again every 20 seconds while this tab is open, and when the window comes back.
  useEffect(() => {
    const t = setInterval(() => { if (document.visibilityState === "visible") void reload(); }, 20_000);
    const on = () => { if (document.visibilityState === "visible") void reload(); };
    document.addEventListener("visibilitychange", on);
    return () => { clearInterval(t); document.removeEventListener("visibilitychange", on); };
  }, [sid]);

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
        {st && <button className="btn primary" disabled={busy} onClick={() => setProtecting(true)}><ShieldCheck size={13} /> Protect SSH</button>}
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
            {st.jails.length === 0 && <div className="empty" style={{ flex: "none" }}><p>No jails are enabled. <button className="link-btn" onClick={() => setProtecting(true)}>Protect SSH</button> turns on the sshd jail with limits you choose.</p></div>}
            <div className="section-title">Banned now{banned ? ` · ${banned}` : ""}</div>
            {banned === 0 ? <p className="faint" style={{ margin: "0 16px", fontSize: 12.5 }}>Nobody is banned right now.</p> : (
              <table className="table">
                <thead><tr><th>Address</th><th>Jail</th><th /></tr></thead>
                <tbody>
                  {st.jails.flatMap((j) => j.banned.map((ip) => (
                    <tr key={`${j.name}:${ip}`}>
                      <td className="mono">{ip}{ip === st.clientIp && <span className="badge amber" style={{ marginLeft: 6 }}>you</span>}</td>
                      <td className="mono faint">{j.name}</td>
                      <td style={{ textAlign: "right" }}><button className="btn" disabled={busy} onClick={() => unban(ip, j.name)}>Unban</button></td>
                    </tr>
                  )))}
                </tbody>
              </table>
            )}
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
      {protecting && <ProtectDialog sid={sid} clientIp={st?.clientIp ?? null} onClose={() => setProtecting(false)} onDone={(x) => { setData(x); setProtecting(false); }} />}
      {unbanAll && (
        <Modal title="Unban an address" subtitle="Removes the address from every jail, for example when you locked yourself out from another machine." onClose={() => setUnbanAll(false)}
          footer={<><button className="btn" onClick={() => setUnbanAll(false)}>Cancel</button><button className="btn primary" disabled={busy || !anyIp.trim()} onClick={async () => { if (await unban(anyIp.trim(), null)) { setUnbanAll(false); setAnyIp(""); } }}>Unban everywhere</button></>}>
          <label className="field"><span>IP address</span><input className="input mono" autoFocus placeholder="203.0.113.7" value={anyIp} onChange={(e) => setAnyIp(e.target.value)} /></label>
        </Modal>
      )}
    </div>
  );
}

const BAN_CHOICES: [number, string][] = [[10, "10 minutes"], [60, "1 hour"], [1440, "1 day"], [10080, "1 week"], [-1, "For good"]];

/** Fail2ban → Protect SSH: how many failed logins, within how long, ban for how long. */
function ProtectDialog({ sid, clientIp, onClose, onDone }: { sid: string; clientIp: string | null; onClose: () => void; onDone: (st: Fail2banStatus) => void }) {
  const [p, setP] = useState<SshProtection | null>(null);
  const [saved, setSaved] = useState(false);
  const [extra, setExtra] = useState("");
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  useEffect(() => {
    api.fail2banProtection(sid).then((x) => {
      setSaved(!!x);
      const base: SshProtection = x ?? { maxRetry: 5, findTimeMins: 10, banTimeMins: 60, increment: true, ignoreSelf: true, recidive: true, ignore: [] };
      // The saved list includes this computer's address when it was kept out; show it as the checkbox instead.
      const ignoreSelf = x ? !!clientIp && x.ignore.includes(clientIp) : true;
      setP({ ...base, ignoreSelf: x ? ignoreSelf : true, ignore: base.ignore.filter((i) => i !== clientIp) });
      setExtra(base.ignore.filter((i) => i !== clientIp).join(" "));
    }, () => setP({ maxRetry: 5, findTimeMins: 10, banTimeMins: 60, increment: true, ignoreSelf: true, recidive: true, ignore: [] }));
  }, [sid]);
  if (!p) return null;
  const set = (x: Partial<SshProtection>) => setP({ ...p, ...x });
  const go = async () => {
    setBusy(true);
    setProblem(null);
    const protection = { ...p, ignore: extra.split(/[\s,]+/).filter(Boolean) };
    try {
      const st = await withSafetyOverride(
        (force) => runJob((pr) => api.fail2banProtect(sid, protection, force, pr), { key: `f2b-protect:${sid}`, title: "Protecting SSH with Fail2ban", sid, done: () => "SSH protection is on" }),
        "this could lock you out",
      );
      if (st) onDone(st);
    } catch (e) {
      setProblem(asApiError(e).message.replace(/^Invalid input: /, ""));
    } finally {
      setBusy(false);
    }
  };
  const banLabel = p.banTimeMins === -1 ? "for good" : BAN_CHOICES.find(([m]) => m === p.banTimeMins)?.[1] ?? `${p.banTimeMins} minutes`;
  return (
    <Modal title="Protect SSH" subtitle="Ban addresses that keep failing to sign in. Written to Orbital's own Fail2ban file (the stock ones aren't changed), checked by fail2ban before it's kept." keepOnOutsideClick onClose={onClose}
      footer={<><button className="btn" onClick={onClose}>Cancel</button><button className="btn primary" disabled={busy} onClick={go}>{busy ? "Applying…" : saved ? "Save" : "Turn on"}</button></>}>
      <div className="route-form">
        <div className="protect-row">
          Ban after
          <input className="input mono" style={{ width: 64 }} type="number" min={2} max={20} value={p.maxRetry} onChange={(e) => set({ maxRetry: Number(e.target.value) || 0 })} aria-label="Failed attempts" />
          failed logins within
          <input className="input mono" style={{ width: 72 }} type="number" min={1} max={1440} value={p.findTimeMins} onChange={(e) => set({ findTimeMins: Number(e.target.value) || 0 })} aria-label="Minutes" />
          minutes
        </div>
        <div className="field">
          <span>Ban for</span>
          <div className="segmented">
            {BAN_CHOICES.map(([m, label]) => <button key={m} className={p.banTimeMins === m ? "on" : ""} onClick={() => set({ banTimeMins: m })}>{label}</button>)}
          </div>
        </div>
        <label className="check"><input type="checkbox" checked={p.increment} disabled={p.banTimeMins === -1} onChange={(e) => set({ increment: e.target.checked })} /> Ban repeat offenders longer each time (doubling, up to a week)</label>
        <label className="check"><input type="checkbox" checked={p.recidive} onChange={(e) => set({ recidive: e.target.checked })} /> Ban addresses banned 5 times in a day for a week (all services)</label>
        <label className="check"><input type="checkbox" checked={p.ignoreSelf} onChange={(e) => set({ ignoreSelf: e.target.checked })} /> Never ban this computer{clientIp ? <> (<span className="mono">{clientIp}</span>)</> : ""}</label>
        <label className="field">
          <span>Also never ban</span>
          <input className="input mono" value={extra} onChange={(e) => setExtra(e.target.value)} placeholder="198.51.100.0/24 203.0.113.5" />
          <span className="hint">Your office or VPN addresses, separated by spaces.</span>
        </label>
        <p className="faint" style={{ margin: 0, fontSize: 12.5 }}>So: {p.maxRetry} wrong passwords or keys within {p.findTimeMins} min from one address and it's blocked {banLabel}.</p>
        {problem && <div className="alert" style={{ whiteSpace: "pre-wrap" }}>{problem}</div>}
      </div>
    </Modal>
  );
}
