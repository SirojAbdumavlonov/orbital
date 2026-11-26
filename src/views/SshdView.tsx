import { AlertTriangle, CheckCircle2, FileText, Info, RefreshCw, Search, ShieldAlert, ShieldCheck, Wand2, XCircle, Zap } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { confirm, Modal } from "../components/ui";
import { api, asApiError, HardenState, SshdCheck } from "../lib/api";
import { useRemote, withSafetyOverride } from "../lib/remote";
import { SessionState, useStore } from "../store";
import { useKept } from "../lib/kept";

const ICON = { bad: XCircle, warn: AlertTriangle, ok: CheckCircle2, info: Info };
const COLOR = { bad: "var(--bad)", warn: "var(--warn)", ok: "var(--ok)", info: "var(--ink-3)" };

/** What Orbital's hardening file can set for a check that isn't fine yet. */
const FIXES: Record<string, { setting: string; value: string; label: string }> = {
  "root-login": { setting: "PermitRootLogin", value: "prohibit-password", label: "root may sign in with a key only" },
  "password-login": { setting: "PasswordAuthentication", value: "no", label: "turn password login off (keys only)" },
  "kbd-interactive": { setting: "KbdInteractiveAuthentication", value: "no", label: "turn keyboard-interactive (PAM password) login off" },
  "empty-passwords": { setting: "PermitEmptyPasswords", value: "no", label: "refuse accounts with an empty password" },
  "max-auth-tries": { setting: "MaxAuthTries", value: "3", label: "3 attempts per connection" },
  x11: { setting: "X11Forwarding", value: "no", label: "turn X11 forwarding off" },
  grace: { setting: "LoginGraceTime", value: "30", label: "30 seconds to sign in" },
};

/**
 * One-click fixes: the chosen settings go into Orbital's own drop-in,
 * checked with sshd -t, then sshd reloads (open connections stay). Changes
 * that would lock this connection out need a typed confirmation.
 */
function HardenDialog({ s, checks, state, onClose, onDone }: { s: SessionState; checks: SshdCheck[]; state: HardenState; onClose: () => void; onDone: (st: HardenState) => void }) {
  const toast = useStore((x) => x.toast);
  const fixable = checks.filter((c) => FIXES[c.id] && c.level !== "ok" && state.current[FIXES[c.id].setting] !== FIXES[c.id].value);
  const risky = (id: string) => id === "password-login" || id === "kbd-interactive";
  const [picked, setPicked] = useState<Set<string>>(() => new Set(fixable.filter((c) => !(risky(c.id) && s.profile.authMethod === "password")).map((c) => c.id)));
  const [busy, setBusy] = useState(false);
  async function apply() {
    const choices: Record<string, string> = { ...state.current };
    for (const id of picked) choices[FIXES[id].setting] = FIXES[id].value;
    setBusy(true);
    try {
      const r = await withSafetyOverride((force) => api.hardenSsh(s.info.id, choices, force), "this could lock you out");
      if (!r) return;
      toast({
        kind: r.notInEffect.length ? "info" : "success",
        title: "SSH server hardened",
        body: r.notInEffect.length
          ? `Saved and reloaded, but another file sets ${r.notInEffect.map(([k, v]) => `${k} (${v})`).join(", ")} first. Look in the list of configuration files.`
          : "sshd -t passed and the SSH server reloaded. This connection stays open.",
      });
      onDone(r.state);
    } catch (e) {
      toast({ kind: "error", title: "Nothing changed", body: asApiError(e).message });
    } finally {
      setBusy(false);
    }
  }
  return (
    <Modal title="Harden the SSH server" subtitle={<>Settings go into <span className="mono">{state.dropin}</span>, read before the other drop-ins.</>} onClose={onClose}
      footer={<><button className="btn" onClick={onClose}>Cancel</button><button className="btn primary" disabled={busy || picked.size === 0} onClick={() => void apply()}>{busy ? "Applying…" : `Apply ${picked.size}`}</button></>}>
      {fixable.length === 0 ? <p className="muted">Everything Orbital can fix here is already fine.</p> : (
        <div className="form">
          {fixable.map((c) => {
            const fx = FIXES[c.id];
            return (
              <label key={c.id} className="check">
                <input type="checkbox" checked={picked.has(c.id)} onChange={(e) => setPicked((p) => { const n = new Set(p); if (e.target.checked) n.add(c.id); else n.delete(c.id); return n; })} />
                <span>
                  <b>{c.title}</b>: {fx.label} <span className="mono faint">({fx.setting} {fx.value})</span>
                  {risky(c.id) && s.profile.authMethod === "password" && <><br /><span className="bad-text" style={{ fontSize: 12 }}>This connection signs in with a password: switch it to a key first (SSH keys tab), or you'll be locked out.</span></>}
                </span>
              </label>
            );
          })}
          <p className="faint" style={{ margin: 0, fontSize: 12 }}>sshd -t checks the result before the reload; if it fails, the file is put back. Remove Orbital's file at any time to undo.</p>
        </div>
      )}
    </Modal>
  );
}

/** The SSH server's real settings, checked against hardening advice, and who is logged in. */
export function SshdView({ s }: { s: SessionState }) {
  const { toast, openFile } = useStore();
  const sid = s.info.id;
  const unit = s.tools?.find((t) => t.id === "sshd")?.unit ?? null;
  const { data: r, error, loading, reload } = useRemote(() => api.sshdReport(sid), [sid]);
  const [q, setQ] = useKept(`${s.info.id}:sshd:q`, "");
  const [reloading, setReloading] = useState(false);
  const [harden, setHarden] = useState<HardenState | null>(null);
  const [hardening, setHardening] = useState(false);
  useEffect(() => { api.hardenState(sid).then(setHarden).catch(() => setHarden(null)); }, [sid, r]);
  const hardenCount = (r?.checks ?? []).filter((c) => FIXES[c.id] && c.level !== "ok" && harden?.current[FIXES[c.id].setting] !== FIXES[c.id].value).length;

  async function unharden() {
    if (!harden) return;
    const ok = await confirm({ title: "Remove Orbital's hardening file?", body: <>Deletes <span className="mono">{harden.dropin}</span> and reloads the SSH server; the other files' settings apply again.</>, confirmLabel: "Remove", danger: true });
    if (!ok) return;
    try {
      setHarden((await api.hardenSsh(sid, {}, false)).state);
      toast({ kind: "success", title: "Hardening file removed" });
      reload();
    } catch (e) {
      toast({ kind: "error", title: "Couldn't remove it", body: asApiError(e).message });
    }
  }

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
        {r?.readable && harden?.includesDropins && <button className="btn primary" disabled={hardenCount === 0} title={hardenCount ? "Fix these settings in one step" : "Nothing left that Orbital can fix"} onClick={() => setHardening(true)}><Wand2 size={13} /> Harden{hardenCount ? ` (${hardenCount})` : ""}</button>}
        <button className="btn" onClick={() => openFile(sid, mainFile)}><FileText size={13} /> Edit sshd_config</button>
        {unit && <button className="btn" disabled={reloading} title="Apply saved changes. sshd -t runs first, and open connections stay up." onClick={reloadSshd}><Zap size={13} className={reloading ? "spin" : ""} /> Reload SSH</button>}
        <button className="icon-btn" aria-label="Refresh" onClick={reload}><RefreshCw size={14} className={loading ? "spin" : ""} /></button>
      </div>
      {error && <div className="empty"><div><h3>Couldn't read the SSH server settings</h3><p>{error.message}</p><button className="btn" onClick={reload}>Try again</button></div></div>}
      {r && !r.testOk && r.testOutput && <div className="alert" style={{ margin: 12 }}><b>sshd -t:</b><pre className="output" style={{ marginTop: 6 }}>{r.testOutput}</pre></div>}
      {harden && Object.keys(harden.current).length > 0 && (
        <div className="notice harden-note">
          <span>Orbital's hardening file sets {Object.entries(harden.current).map(([k, v]) => <span key={k} className="mono"> {k} {v};</span>)}</span>
          <button className="btn" onClick={() => void unharden()}>Remove it</button>
        </div>
      )}
      {r?.readable && harden && !harden.includesDropins && <div className="notice">This sshd_config doesn't read <span className="mono">/etc/ssh/sshd_config.d</span> (OpenSSH before 8.2, or the Include line was removed), so one-click hardening isn't available. Change the settings in sshd_config in the editor.</div>}
      {hardening && r && harden && <HardenDialog s={s} checks={r.checks} state={harden} onClose={() => setHardening(false)} onDone={(st) => { setHarden(st); setHardening(false); reload(); }} />}
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
