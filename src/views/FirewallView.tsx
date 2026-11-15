import { Pencil, Plus, Power, RefreshCw, ShieldAlert, ShieldCheck, Trash2 } from "lucide-react";
import { useState } from "react";
import { confirm, Modal } from "../components/ui";
import { api, asApiError, UfwRule, UfwRuleInput, UfwStatus } from "../lib/api";
import { useRemote, withSafetyOverride } from "../lib/remote";
import { SessionState, useStore } from "../store";

const ACTION_LABEL: Record<UfwRule["action"], string> = { ALLOW: "Allow", LIMIT: "Limit", DENY: "Deny", REJECT: "Reject" };

const PRESETS: { label: string; rule: Partial<UfwRuleInput> }[] = [
  { label: "SSH", rule: { action: "limit", port: "22", protocol: "tcp", comment: "SSH" } },
  { label: "HTTP", rule: { port: "80", protocol: "tcp", comment: "web" } },
  { label: "HTTPS", rule: { port: "443", protocol: "tcp", comment: "web" } },
  { label: "HTTP + HTTPS", rule: { port: "80,443", protocol: "tcp", comment: "web" } },
  { label: "PostgreSQL", rule: { port: "5432", protocol: "tcp", comment: "PostgreSQL", address: "" } },
  { label: "MySQL", rule: { port: "3306", protocol: "tcp", comment: "MySQL", address: "" } },
];

const blank = (): UfwRuleInput => ({ action: "allow", direction: "in", port: "", protocol: "tcp", address: null, app: null, comment: null });

const strip = (v: string) => v.replace(" (v6)", "");
/** The IPv6 copy ufw keeps of an IPv4 rule added for "anywhere". */
const twinOf = (r: UfwRule, all: UfwRule[]) =>
  all.find((x) => x.number !== r.number && x.v6 !== r.v6 && strip(x.to) === strip(r.to) && x.action === r.action && x.direction === r.direction && strip(x.from) === strip(r.from) && x.comment === r.comment);

/** A live rule back in form shape, or null when the form can't express it (interfaces, addresses as target). */
function toInput(r: UfwRule, apps: string[]): UfwRuleInput | null {
  // Outgoing and forwarding rules, and rules bound to an interface, are edited in the terminal.
  const to = strip(r.to);
  if (to.includes(" on ") || r.direction !== "IN") return null;
  const from = strip(r.from);
  const base: UfwRuleInput = {
    ...blank(),
    action: r.action.toLowerCase() as UfwRuleInput["action"],
    comment: r.comment,
    address: from === "Anywhere" ? null : from,
  };
  if (apps.includes(to)) return { ...base, app: to, port: null, protocol: "any" };
  const m = to.match(/^([\d,:]+)(?:\/(tcp|udp))?$/);
  if (!m) return null;
  return { ...base, port: m[1], protocol: (m[2] as "tcp" | "udp") ?? "any" };
}

function RuleDialog({ status, initial, title, onClose, onSave }: { status: UfwStatus; initial: UfwRuleInput; title: string; onClose: () => void; onSave: (r: UfwRuleInput) => Promise<boolean> }) {
  const [r, setR] = useState<UfwRuleInput>(initial);
  const [target, setTarget] = useState<"port" | "app">(initial.app ? "app" : "port");
  const [anywhere, setAnywhere] = useState(!initial.address);
  const [busy, setBusy] = useState(false);
  const set = (p: Partial<UfwRuleInput>) => setR((x) => ({ ...x, ...p }));
  const save = async () => {
    setBusy(true);
    const rule: UfwRuleInput = {
      ...r,
      port: target === "port" ? (r.port ?? "").trim() || null : null,
      app: target === "app" ? r.app : null,
      protocol: target === "app" ? "any" : r.protocol,
      address: anywhere ? null : (r.address ?? "").trim() || null,
      comment: (r.comment ?? "").trim() || null,
    };
    const ok = await onSave(rule);
    setBusy(false);
    if (ok) onClose();
  };
  const isIn = r.direction === "in";
  return (
    <Modal title={title} subtitle="ufw checks rules from the top; the first one that matches decides." onClose={onClose}
      footer={<><button className="btn" onClick={onClose}>Cancel</button><button className="btn primary" disabled={busy || (target === "port" ? !r.port : !r.app) || (!anywhere && !r.address)} onClick={save}>{busy ? "Saving…" : "Save rule"}</button></>}>
      <div className="form">
        <div className="chips">
          {PRESETS.map((p) => (
            <button key={p.label} className="chip" onClick={() => { setTarget("port"); setAnywhere(p.rule.address === undefined); setR({ ...blank(), ...p.rule, address: p.rule.address ?? null }); }}>{p.label}</button>
          ))}
        </div>
        <div className="field"><span>Traffic</span>
          <div className="segmented">
            <button className={isIn ? "on" : ""} onClick={() => set({ direction: "in" })}>Incoming</button>
            <button className={!isIn ? "on" : ""} onClick={() => set({ direction: "out", action: r.action === "limit" ? "allow" : r.action })}>Outgoing</button>
          </div>
        </div>
        <div className="field"><span>What to do</span>
          <div className="segmented">
            {(["allow", "limit", "deny", "reject"] as const).filter((a) => isIn || a !== "limit").map((a) => (
              <button key={a} className={r.action === a ? "on" : ""} onClick={() => set({ action: a })}>{a[0].toUpperCase() + a.slice(1)}</button>
            ))}
          </div>
          <small className="hint">{{ allow: "Let the traffic through.", limit: "Allow, but block an address that opens 6 or more connections in 30 seconds. Good for SSH.", deny: "Drop it silently.", reject: "Refuse it, so the other side knows at once." }[r.action]}</small>
        </div>
        <div className="field"><span>Applies to</span>
          <div className="segmented">
            <button className={target === "port" ? "on" : ""} onClick={() => setTarget("port")}>Port</button>
            <button className={target === "app" ? "on" : ""} disabled={status.apps.length === 0} title={status.apps.length ? "" : "No application profiles installed"} onClick={() => { setTarget("app"); set({ app: r.app ?? status.apps[0] }); }}>Application</button>
          </div>
        </div>
        {target === "port" ? (
          <div className="row2">
            <label className="field"><span>Port</span>
              <input className="input mono" placeholder="22, 80,443 or 6000:6007" value={r.port ?? ""} onChange={(e) => set({ port: e.target.value })} autoFocus />
            </label>
            <div className="field"><span>Protocol</span>
              <div className="segmented">
                {(["tcp", "udp", "any"] as const).map((p) => <button key={p} className={r.protocol === p ? "on" : ""} onClick={() => set({ protocol: p })}>{p === "any" ? "Both" : p.toUpperCase()}</button>)}
              </div>
            </div>
          </div>
        ) : (
          <label className="field"><span>Application profile</span>
            <select className="input" value={r.app ?? ""} onChange={(e) => set({ app: e.target.value })}>
              {status.apps.map((a) => <option key={a}>{a}</option>)}
            </select>
          </label>
        )}
        <div className="field"><span>{isIn ? "From" : "To"}</span>
          <div className="segmented">
            <button className={anywhere ? "on" : ""} onClick={() => setAnywhere(true)}>Anywhere</button>
            <button className={!anywhere ? "on" : ""} onClick={() => setAnywhere(false)}>A specific address or network</button>
          </div>
          {!anywhere && <input className="input mono" placeholder="203.0.113.7 or 10.0.0.0/8" value={r.address ?? ""} onChange={(e) => set({ address: e.target.value })} />}
        </div>
        <label className="field"><span>Note (optional)</span>
          <input className="input" placeholder="What this rule is for" value={r.comment ?? ""} onChange={(e) => set({ comment: e.target.value })} />
        </label>
      </div>
    </Modal>
  );
}

/** ufw rules: see, add, change and delete them, and switch the firewall on or off without locking yourself out. */
export function FirewallView({ s }: { s: SessionState }) {
  const { toast } = useStore();
  const sid = s.info.id;
  const { data: st, setData, error, loading, reload } = useRemote(() => api.ufwStatus(sid), [sid]);
  const [dialog, setDialog] = useState<{ title: string; initial: UfwRuleInput; old?: UfwRule[] } | null>(null);
  const [busy, setBusy] = useState(false);
  const sshPort = s.profile.port;

  async function change(what: string, run: (force: boolean) => Promise<UfwStatus>, done: string): Promise<boolean> {
    setBusy(true);
    try {
      const next = await withSafetyOverride(run, what);
      if (!next) return false;
      setData(next);
      toast({ kind: "success", title: done });
      return true;
    } catch (e) {
      toast({ kind: "error", title: `Couldn't ${what}`, body: asApiError(e).message });
      reload();
      return false;
    } finally {
      setBusy(false);
    }
  }

  async function remove(r: UfwRule) {
    if (!st) return;
    const twin = twinOf(r, st.rules);
    const ok = await confirm({
      title: `Delete rule ${r.number}?`,
      body: <>
        <div className="mono" style={{ margin: "4px 0 8px" }}>{r.raw}</div>
        {twin && <div>Its IPv6 copy (rule {twin.number}) is deleted too.</div>}
      </>,
      confirmLabel: "Delete rule",
      danger: true,
    });
    if (!ok) return;
    const refs = [r, ...(twin ? [twin] : [])].map(({ number, raw }) => ({ number, raw }));
    await change("delete the rule", (force) => api.ufwDelete(sid, refs, force), "Rule deleted");
  }

  async function toggle() {
    if (!st) return;
    if (st.active) {
      const ok = await confirm({ title: "Turn the firewall off?", body: "Every port with a listening program becomes reachable from the internet until you turn it on again.", confirmLabel: "Turn off", danger: true });
      if (!ok) return;
    }
    await change(st.active ? "turn the firewall off" : "turn the firewall on", (force) => api.ufwSetEnabled(sid, !st.active, force), st.active ? "Firewall is off" : "Firewall is on");
  }

  const isSsh = (r: UfwRule) => r.direction === "IN" && (strip(r.to).split(/[ /]/)[0].split(",").includes(String(sshPort)) || strip(r.to) === "OpenSSH");

  return (
    <div className="view">
      <div className="toolbar">
        {st && (st.active
          ? <span className="badge ok"><ShieldCheck size={12} /> Firewall is on</span>
          : <span className="badge bad"><ShieldAlert size={12} /> Firewall is off</span>)}
        {st?.defaultIncoming && <span className="faint">Incoming by default: <b className="muted">{st.defaultIncoming}</b> · outgoing: <b className="muted">{st.defaultOutgoing ?? "—"}</b></span>}
        <span className="spacer" />
        {st && <button className="btn" disabled={busy} onClick={toggle}><Power size={13} /> {st.active ? "Turn off" : "Turn on"}</button>}
        {st && <button className="btn primary" disabled={busy} onClick={() => setDialog({ title: "Add a rule", initial: blank() })}><Plus size={13} /> Add rule</button>}
        <button className="icon-btn" aria-label="Refresh" onClick={reload}><RefreshCw size={14} className={loading ? "spin" : ""} /></button>
      </div>
      {error && <div className="empty"><div><h3>Couldn't read the firewall</h3><p>{error.message}</p><button className="btn" onClick={reload}>Try again</button></div></div>}
      {st && !st.active && (
        <div className="scroll">
          <div className="notice">
            The firewall is off, so every listening port is reachable. The rules below take effect when you turn it on. Orbital checks first that they still let SSH in on port {sshPort}.
          </div>
          <div className="section-title">Rules waiting to apply ({st.added.length})</div>
          <table className="table"><tbody>
            {st.added.map((a, i) => <tr key={i}><td className="mono">ufw {a}</td></tr>)}
          </tbody></table>
          {st.added.length === 0 && <div className="empty" style={{ flex: "none" }}><p>No rules yet. Add one for SSH before turning the firewall on.</p></div>}
        </div>
      )}
      {st?.active && (
        <div className="scroll">
          <table className="table">
            <thead><tr><th style={{ width: 40 }}>#</th><th>Port or app</th><th>Action</th><th>From</th><th>Note</th><th style={{ width: 70 }} /></tr></thead>
            <tbody>
              {st.rules.map((r) => {
                const editable = toInput(r, st.apps);
                return (
                  <tr key={r.number}>
                    <td className="num faint">{r.number}</td>
                    <td className="mono">{strip(r.to)}{r.v6 && <span className="faint"> IPv6</span>}{isSsh(r) && <span className="badge amber" style={{ marginLeft: 6 }}>SSH</span>}</td>
                    <td><span className={`badge ${r.action === "ALLOW" || r.action === "LIMIT" ? "ok" : "bad"}`}>{ACTION_LABEL[r.action]} {r.direction === "IN" ? "in" : r.direction === "OUT" ? "out" : "forward"}</span></td>
                    <td className="mono muted">{strip(r.from)}</td>
                    <td className="muted">{r.comment ?? ""}</td>
                    <td className="nowrap" style={{ textAlign: "right" }}>
                      <button className="icon-btn" aria-label={`Edit rule ${r.number}`} title={editable ? "Edit" : "This rule uses options the form doesn't cover; edit it in the terminal"} disabled={!editable || busy}
                        onClick={() => editable && setDialog({ title: `Edit rule ${r.number}`, initial: editable, old: [r, ...[twinOf(r, st.rules)].filter((x): x is UfwRule => !!x)] })}><Pencil size={13} /></button>
                      <button className="icon-btn" aria-label={`Delete rule ${r.number}`} title="Delete" disabled={busy} onClick={() => remove(r)}><Trash2 size={13} /></button>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
          {st.rules.length === 0 && <div className="empty" style={{ flex: "none" }}><p>No rules: everything follows the default policy ({st.defaultIncoming ?? "deny"} incoming).</p></div>}
          <p className="footnote">Rules are checked from the top. Traffic no rule matches follows the default policy. Changes that would block SSH on port {sshPort} for this connection are stopped before they are sent.</p>
        </div>
      )}
      {dialog && st && (
        <RuleDialog status={st} title={dialog.title} initial={dialog.initial} onClose={() => setDialog(null)}
          onSave={(rule) => dialog.old
            ? change("change the rule", (force) => api.ufwUpdate(sid, dialog.old!.map(({ number, raw }) => ({ number, raw })), rule, force), "Rule changed")
            : change("add the rule", (force) => api.ufwAdd(sid, rule, force), st.active ? "Rule added" : "Rule saved; it applies when the firewall is on")} />
      )}
    </div>
  );
}
