import { CheckCircle2, CircleDashed, Cloud, Globe, Pencil, Plus, RefreshCw, Search, Trash2, XCircle } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { confirm, DrawerClose, Modal } from "../components/ui";
import { api, asApiError, DnsAccount, DnsCheck, DnsRecord, DnsRecordInput, DnsType, DnsZone } from "../lib/api";
import { isSsh, useStore } from "../store";

const TYPES: DnsType[] = ["A", "AAAA", "CNAME", "TXT", "MX"];
const TYPE_HINT: Record<DnsType, string> = {
  A: "Points the name at a server's IPv4 address.",
  AAAA: "Points the name at a server's IPv6 address.",
  CNAME: "Makes the name an alias of another name (for example a hosting provider's).",
  TXT: "Text, used to prove you own the domain (Google, mail SPF/DKIM/DMARC, …).",
  MX: "Where e-mail for the domain is delivered.",
};
const TTLS: [number, string][] = [[1, "Automatic"], [60, "1 minute"], [300, "5 minutes"], [3600, "1 hour"], [86400, "1 day"]];
const ttlLabel = (t: number) => TTLS.find(([v]) => v === t)?.[1] ?? `${t} s`;

/** The answer a record should produce, in the form the check compares. Empty: any answer. */
function expectedFor(r: { type: string; content: string; proxied: boolean; priority: number | null }): string[] {
  if (r.proxied) return []; // Cloudflare answers with its own addresses.
  return r.type === "MX" ? [`${r.priority ?? 10} ${r.content}`] : [r.content];
}
const checkable = (t: string): t is DnsType => (TYPES as string[]).includes(t);
/** "www.example.com" in zone "example.com" → "www"; the zone itself → "@". */
const short = (name: string, zone: string) => (name === zone ? "@" : name.endsWith(`.${zone}`) ? name.slice(0, -zone.length - 1) : name);

type Sel = { kind: "check" } | { kind: "zone"; account: DnsAccount; zone: DnsZone };

/** DNS: records at your provider, and whether the world sees them yet. */
export function DomainsView() {
  const toast = useStore((x) => x.toast);
  const [accounts, setAccounts] = useState<DnsAccount[]>([]);
  const [zones, setZones] = useState<Record<string, DnsZone[] | { error: string }>>({});
  const [sel, setSel] = useState<Sel>({ kind: "check" });
  const [adding, setAdding] = useState(false);

  const loadZones = (a: DnsAccount) =>
    api.dnsZones(a.id).then((z) => setZones((m) => ({ ...m, [a.id]: z }))).catch((e) => setZones((m) => ({ ...m, [a.id]: { error: asApiError(e).message } })));
  const load = () => api.dnsAccounts().then((list) => { setAccounts(list); list.forEach(loadZones); }).catch(() => {});
  useEffect(() => { load(); }, []);

  async function remove(a: DnsAccount) {
    const ok = await confirm({ title: `Remove ${a.name}?`, body: "Orbital forgets the account and deletes its saved API token. Your DNS records at Cloudflare stay as they are.", confirmLabel: "Remove account", danger: true });
    if (!ok) return;
    try {
      await api.removeDnsAccount(a.id);
      if (sel.kind === "zone" && sel.account.id === a.id) setSel({ kind: "check" });
      load();
    } catch (e) {
      toast({ kind: "error", title: "Couldn't remove the account", body: asApiError(e).message });
    }
  }

  return (
    <div className="view">
      <div className="domains">
        <nav className="domains-nav">
          <button className={`nav-item ${sel.kind === "check" ? "on" : ""}`} onClick={() => setSel({ kind: "check" })}><Search size={14} /> Check a domain</button>
          {accounts.map((a) => {
            const z = zones[a.id];
            return (
              <div key={a.id} className="domains-account">
                <div className="domains-account-head">
                  <Cloud size={13} /> <span className="grow" title={`Cloudflare: ${a.name}`}>{a.name}</span>
                  <button className="icon-btn" aria-label={`Reload ${a.name}`} onClick={() => loadZones(a)}><RefreshCw size={12} /></button>
                  <button className="icon-btn" aria-label={`Remove ${a.name}`} onClick={() => remove(a)}><Trash2 size={12} /></button>
                </div>
                {!z && <div className="faint domains-note">Loading domains…</div>}
                {z && "error" in z && <div className="domains-note bad-text" title={z.error}>{z.error}</div>}
                {z && !("error" in z) && z.length === 0 && <div className="faint domains-note">This token can't see any domains.</div>}
                {z && !("error" in z) && z.map((zone) => (
                  <button key={zone.id} className={`nav-item mono ${sel.kind === "zone" && sel.zone.id === zone.id ? "on" : ""}`} onClick={() => setSel({ kind: "zone", account: a, zone })}>
                    <Globe size={13} /> <span className="grow">{zone.name}</span>
                    {zone.status !== "active" && <span className="badge warn">{zone.status}</span>}
                  </button>
                ))}
              </div>
            );
          })}
          <button className="btn domains-add" onClick={() => setAdding(true)}><Plus size={13} /> Connect Cloudflare</button>
        </nav>
        {sel.kind === "check" ? <CheckPage hasAccounts={accounts.length > 0} onConnect={() => setAdding(true)} /> : <ZonePage key={sel.zone.id} account={sel.account} zone={sel.zone} />}
      </div>
      {adding && <AccountDialog onClose={() => setAdding(false)} onAdded={(a) => { setAdding(false); load(); toast({ kind: "success", title: `Connected ${a.name}` }); }} />}
    </div>
  );
}

function AccountDialog({ onClose, onAdded }: { onClose: () => void; onAdded: (a: DnsAccount) => void }) {
  const [name, setName] = useState("Cloudflare");
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  async function save() {
    setBusy(true);
    setError(null);
    try {
      onAdded(await api.addDnsAccount(name, "cloudflare", token));
    } catch (e) {
      setError(asApiError(e).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Modal title="Connect Cloudflare" subtitle="Manage the DNS records of your domains on Cloudflare from Orbital." onClose={onClose}
      footer={<><button className="btn" onClick={onClose}>Cancel</button><button className="btn primary" disabled={busy || !token.trim() || !name.trim()} onClick={save}>{busy ? "Checking the token…" : "Connect"}</button></>}>
      <div className="form">
        <ol className="steps">
          <li>Open <span className="mono">dash.cloudflare.com/profile/api-tokens</span> and click <b>Create Token</b>.</li>
          <li>Use the <b>Edit zone DNS</b> template and choose the domains Orbital may change (or all of them).</li>
          <li>Create the token and paste it below. Cloudflare shows it only once.</li>
        </ol>
        <label className="field"><span>Name in Orbital</span><input className="input" value={name} onChange={(e) => setName(e.target.value)} /></label>
        <label className="field"><span>API token</span>
          <input className="input mono" type="password" autoFocus value={token} onChange={(e) => setToken(e.target.value)} onKeyDown={(e) => e.key === "Enter" && token.trim() && save()} />
          <small className="hint">Kept in Orbital's encrypted vault on this computer (behind Touch ID when that is on), never in the activity log. Orbital checks it by listing your domains before saving it.</small>
        </label>
        {error && <div className="notice bad-text" style={{ margin: 0 }}>{error}</div>}
      </div>
    </Modal>
  );
}

/** Check any name with the public resolvers: for domains at any provider. */
function CheckPage({ hasAccounts, onConnect }: { hasAccounts: boolean; onConnect: () => void }) {
  const [name, setName] = useState("");
  const [type, setType] = useState<DnsType>("A");
  const [expected, setExpected] = useState("");
  const [target, setTarget] = useState<{ name: string; type: DnsType; expected: string[] } | null>(null);
  const suggestions = useServerAddresses();
  const run = () => name.trim() && setTarget({ name: name.trim(), type, expected: expected.trim() ? [expected.trim()] : [] });
  return (
    <div className="domains-main scroll">
      <div className="settings">
        <h3 className="settings-title">Check a domain</h3>
        <p className="muted" style={{ margin: "0 0 12px" }}>
          Asks public DNS resolvers what they answer for a name, the same answer browsers and Let's Encrypt get. Works for domains at any provider: add the record there, then check it here until it shows up.
        </p>
        <div className="setting check-form">
          <label className="field grow"><span>Name</span><input className="input mono" placeholder="www.example.com" value={name} onChange={(e) => setName(e.target.value)} onKeyDown={(e) => e.key === "Enter" && run()} /></label>
          <label className="field"><span>Type</span>
            <select className="input" value={type} onChange={(e) => setType(e.target.value as DnsType)}>{TYPES.map((t) => <option key={t}>{t}</option>)}</select>
          </label>
          <label className="field grow"><span>Should answer (optional)</span><input className="input mono" placeholder={type === "A" ? "203.0.113.10" : type === "MX" ? "10 mail.example.com" : ""} value={expected} onChange={(e) => setExpected(e.target.value)} onKeyDown={(e) => e.key === "Enter" && run()} /></label>
          <button className="btn primary" disabled={!name.trim()} onClick={run}>Check</button>
        </div>
        {(type === "A" || type === "AAAA") && suggestions.length > 0 && (
          <div className="chips" style={{ margin: "4px 0 8px" }}>
            <span className="faint">Your servers:</span>
            {suggestions.filter((s) => (type === "A") !== s.address.includes(":")).map((s) => (
              <button key={s.label + s.address} className={`chip ${expected === s.address ? "on" : ""}`} onClick={() => setExpected(s.address)}>{s.label} · {s.address}</button>
            ))}
          </div>
        )}
        {target && <CheckPanel key={`${target.name}|${target.type}|${target.expected.join()}`} {...target} keepChecking={target.expected.length > 0} />}
        {!hasAccounts && (
          <div className="setting" style={{ marginTop: 18 }}>
            <div className="setting-text"><b>Domains on Cloudflare?</b><small>Connect your account to add and change records here, then watch them go live.</small></div>
            <div className="setting-control"><button className="btn" onClick={onConnect}><Cloud size={13} /> Connect Cloudflare</button></div>
          </div>
        )}
      </div>
    </div>
  );
}

/** Public addresses of the connected SSH servers, for "point this name at my server". */
function useServerAddresses() {
  const sessions = useStore((x) => x.sessions);
  const [out, setOut] = useState<{ label: string; address: string }[]>([]);
  const ids = Object.values(sessions).filter((s) => isSsh(s)).map((s) => s.info.id).join();
  useEffect(() => {
    let live = true;
    const list = Object.values(useStore.getState().sessions).filter((s) => isSsh(s));
    Promise.all(list.map((s) => api.serverPublicAddresses(s.info.id).then((a) => a.map((address) => ({ label: s.profile.name, address }))).catch(() => [])))
      .then((r) => live && setOut(r.flat()));
    return () => { live = false; };
  }, [ids]);
  return out;
}

const POLL_SECS = 10;
const POLL_FOR_MIN = 15;

/** What each resolver answers, checked again every 10 s until it matches (for up to 15 min). */
export function CheckPanel({ name, type, expected, keepChecking, note }: { name: string; type: DnsType; expected: string[]; keepChecking: boolean; note?: string }) {
  const [check, setCheck] = useState<DnsCheck | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tries, setTries] = useState(0);
  const [polling, setPolling] = useState(keepChecking);
  const [next, setNext] = useState(0);
  const [running, setRunning] = useState(true);
  const started = useRef(Date.now());
  const run = async () => {
    setRunning(true);
    try {
      const c = await api.dnsCheck(name, type, expected);
      setCheck(c);
      setError(null);
      if (c.verified) setPolling(false);
    } catch (e) {
      setError(asApiError(e).message);
      setPolling(false);
    } finally {
      setTries((n) => n + 1);
      setNext(POLL_SECS);
      setRunning(false);
    }
  };
  useEffect(() => { run(); }, []);
  useEffect(() => {
    if (!polling) return;
    const t = setInterval(() => {
      if (Date.now() - started.current > POLL_FOR_MIN * 60_000) {
        setPolling(false);
        return;
      }
      setNext((n) => {
        if (n <= 1) { run(); return POLL_SECS; }
        return n - 1;
      });
    }, 1000);
    return () => clearInterval(t);
  }, [polling]);

  const want = expected.length ? expected.join(", ") : "any answer";
  // Every resolver that answered says the name doesn't exist at all.
  const missing = !!check && check.answers.some((a) => !a.error) && check.answers.every((a) => a.error || a.status === "NXDOMAIN");
  const title = check?.verified
    ? expected.length ? "Live: every resolver answers correctly" : "Resolves: every resolver answers"
    : running && !polling ? "Checking…"
    : polling ? "Waiting for DNS…"
    : error ? "Couldn't check"
    : missing ? "This name doesn't exist in DNS yet"
    : "Not live yet";
  return (
    <div className={`dns-check ${check?.verified ? "ok" : ""}`}>
      <div className="dns-check-head">
        {check?.verified ? <CheckCircle2 size={16} className="ok-text" /> : polling || running ? <CircleDashed size={16} className="spin muted" /> : <XCircle size={16} className="bad-text" />}
        <div className="grow">
          <b>{title}</b>
          <small className="mono">{type} {name} → {want}</small>
        </div>
        {!check?.verified && (polling
          ? <span className="faint">checked {tries}× · next in {next}s</span>
          : <button className="btn" disabled={running} onClick={() => { started.current = Date.now(); setPolling(keepChecking || expected.length > 0); run(); }}><RefreshCw size={13} className={running ? "spin" : ""} /> {running ? "Checking…" : "Check again"}</button>)}
      </div>
      {note && <p className="footnote">{note}</p>}
      {error && <div className="notice bad-text">{error}</div>}
      {check && (
        <table className="table dns-answers">
          <tbody>
            {check.answers.map((a) => (
              <tr key={a.resolver}>
                <td className="nowrap">{a.ok ? <CheckCircle2 size={13} className="ok-text" /> : <CircleDashed size={13} className="faint" />} {a.resolver}</td>
                <td className="mono">{a.error ? <span className="bad-text">{a.error}</span> : a.values.length ? a.values.join("\n") : <span className="faint">{a.status === "NXDOMAIN" ? "no such name yet" : `no ${type} record`}</span>}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {!check?.verified && check && !running && (
        <p className="footnote">
          {missing
            ? "Neither resolver knows this name. For a domain itself, check that it is registered and that its nameservers are set at the registrar (that can take a few hours). For a name under it, add the record at your DNS provider."
            : "New records usually appear within a minute or two. If the name existed before, resolvers keep the old answer until its TTL runs out."}
          {!missing && !polling && tries > 1 && " Still nothing after 15 minutes? Check that the domain's nameservers point at your DNS provider."}
        </p>
      )}
    </div>
  );
}

function ZonePage({ account, zone }: { account: DnsAccount; zone: DnsZone }) {
  const toast = useStore((x) => x.toast);
  const [records, setRecords] = useState<DnsRecord[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [q, setQ] = useState("");
  const [selId, setSelId] = useState<string | null>(null);
  const [editing, setEditing] = useState<DnsRecord | "new" | null>(null);
  /** Records saved in this visit: their drawer keeps checking until they are live. */
  const [watch, setWatch] = useState<Set<string>>(new Set());

  const load = async () => {
    setLoading(true);
    try {
      setRecords(await api.dnsRecords(account.id, zone.id));
      setError(null);
    } catch (e) {
      setError(asApiError(e).message);
    } finally {
      setLoading(false);
    }
  };
  useEffect(() => { load(); }, [zone.id]);

  const shown = useMemo(() => {
    const t = q.trim().toLowerCase();
    return (records ?? []).filter((r) => !t || `${r.type} ${r.name} ${r.content}`.toLowerCase().includes(t));
  }, [records, q]);
  const cur = records?.find((r) => r.id === selId) ?? null;

  async function remove(r: DnsRecord) {
    const ok = await confirm({ title: `Delete ${r.type} ${r.name}?`, body: `It points at ${r.content}. Anything that uses this name stops reaching it once resolvers forget it.`, confirmLabel: "Delete record", danger: true });
    if (!ok) return;
    try {
      await api.deleteDnsRecord(account.id, zone.id, r.id, `${r.type} ${r.name}`);
      setSelId(null);
      load();
    } catch (e) {
      toast({ kind: "error", title: "Couldn't delete the record", body: asApiError(e).message });
    }
  }

  return (
    <div className="domains-main">
      <div className="toolbar">
        <b className="mono">{zone.name}</b>
        {zone.status === "active" ? <span className="badge ok">active</span> : <span className="badge warn">{zone.status}</span>}
        <span className="spacer" />
        <div className="search"><Search size={14} /><input className="input" placeholder="Filter records" value={q} onChange={(e) => setQ(e.target.value)} /></div>
        <button className="btn primary" onClick={() => setEditing("new")}><Plus size={13} /> Add record</button>
        <button className="icon-btn" aria-label="Refresh" onClick={load}><RefreshCw size={14} className={loading ? "spin" : ""} /></button>
      </div>
      {zone.status !== "active" && (
        <div className="notice">
          Cloudflare isn't answering for this domain yet. At the registrar where you bought it, set its nameservers to{" "}
          <span className="mono">{zone.nameServers.join(", ") || "the ones Cloudflare shows"}</span>. Records you add here work once that change has gone through (usually within a few hours).
        </div>
      )}
      {error && <div className="empty"><div><h3>Couldn't read the records</h3><p>{error}</p><button className="btn" onClick={load}>Try again</button></div></div>}
      {records && !error && (
        <div className={`split ${cur ? "" : "no-drawer"}`}>
          <div className="scroll">
            <table className="table">
              <thead><tr><th>Type</th><th>Name</th><th>Content</th><th>TTL</th><th>Proxy</th></tr></thead>
              <tbody>
                {shown.map((r) => (
                  <tr key={r.id} className={`click ${selId === r.id ? "sel" : ""}`} onClick={() => setSelId(r.id)}>
                    <td><span className="badge">{r.type}</span></td>
                    <td className="mono"><div className="cell-clip" style={{ maxWidth: 260 }} title={r.name}>{short(r.name, zone.name)}</div></td>
                    <td className="mono"><div className="cell-clip" style={{ maxWidth: 320 }} title={r.content}>{r.priority != null && r.type === "MX" ? `${r.priority} ` : ""}{r.content}</div></td>
                    <td className="faint nowrap">{ttlLabel(r.ttl)}</td>
                    <td>{r.proxiable ? (r.proxied ? <span className="badge amber" title="Traffic goes through Cloudflare">Proxied</span> : <span className="faint">DNS only</span>) : null}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            {records.length === 0 && <div className="empty"><div><h3>No records yet</h3><p>Add an A record to point this domain at one of your servers.</p><button className="btn primary" onClick={() => setEditing("new")}><Plus size={13} /> Add record</button></div></div>}
          </div>
          {cur && (
          <aside className="drawer">
              <>
                <div className="drawer-head">
                  <h3 className="mono" style={{ wordBreak: "break-all" }}>{cur.name}</h3>
                  <div className="muted">{cur.type} record</div>
                  <DrawerClose onClose={() => setSelId(null)} />
                </div>
                <div className="actions">
                  {checkable(cur.type) && <button className="btn" onClick={() => setEditing(cur)}><Pencil size={13} /> Edit</button>}
                  <button className="btn danger" onClick={() => remove(cur)}><Trash2 size={13} /> Delete</button>
                </div>
                <dl className="kv">
                  <dt>Content</dt><dd className="mono">{cur.content}</dd>
                  {cur.priority != null && <><dt>Priority</dt><dd>{cur.priority}</dd></>}
                  <dt>TTL</dt><dd>{ttlLabel(cur.ttl)}</dd>
                  {cur.proxiable && <><dt>Proxy</dt><dd>{cur.proxied ? "Through Cloudflare (hides the server's address)" : "DNS only"}</dd></>}
                  {cur.comment && <><dt>Comment</dt><dd>{cur.comment}</dd></>}
                </dl>
                <div className="drawer-section">
                  {checkable(cur.type)
                    ? <CheckPanel key={`${cur.id}|${cur.content}|${cur.proxied}`} name={cur.name.replace(/^\*\./, "wildcard-check.")} type={cur.type} expected={expectedFor(cur)} keepChecking={watch.has(cur.id)}
                        note={cur.proxied ? "Proxied: resolvers answer with Cloudflare's addresses, not your server's, so any answer counts." : cur.name.startsWith("*.") ? "A wildcard: checked with a made-up name under it." : undefined} />
                    : <p className="footnote">Orbital checks A, AAAA, CNAME, TXT and MX records.</p>}
                </div>
              </>
          </aside>
          )}
        </div>
      )}
      {editing && (
        <RecordDialog account={account} zone={zone} record={editing === "new" ? null : editing} onClose={() => setEditing(null)}
          onSaved={(r) => {
            setEditing(null);
            setWatch((w) => new Set(w).add(r.id));
            setRecords((list) => list ? [...list.filter((x) => x.id !== r.id), r] : [r]);
            setSelId(r.id);
            toast({ kind: "success", title: `${r.type} ${r.name} saved`, body: "Checking until the world sees it." });
          }} />
      )}
    </div>
  );
}

function RecordDialog({ account, zone, record, onClose, onSaved }: { account: DnsAccount; zone: DnsZone; record: DnsRecord | null; onClose: () => void; onSaved: (r: DnsRecord) => void }) {
  const [f, setF] = useState<DnsRecordInput>(() => record && checkable(record.type)
    ? { type: record.type, name: short(record.name, zone.name), content: record.content, ttl: record.ttl, proxied: record.proxied, priority: record.priority }
    : { type: "A", name: "", content: "", ttl: 1, proxied: false, priority: 10 });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const servers = useServerAddresses();
  const set = <K extends keyof DnsRecordInput>(k: K, v: DnsRecordInput[K]) => setF((x) => ({ ...x, [k]: v }));
  const proxiable = f.type === "A" || f.type === "AAAA" || f.type === "CNAME";
  const full = !f.name.trim() || f.name.trim() === "@" ? zone.name : f.name.trim().endsWith(zone.name) ? f.name.trim() : `${f.name.trim()}.${zone.name}`;
  const ips = servers.filter((s) => (f.type === "A") !== s.address.includes(":"));

  async function save() {
    setBusy(true);
    setError(null);
    try {
      const input = { ...f, proxied: proxiable && f.proxied, priority: f.type === "MX" ? f.priority ?? 10 : null };
      onSaved(record ? await api.updateDnsRecord(account.id, zone.id, zone.name, record.id, input) : await api.createDnsRecord(account.id, zone.id, zone.name, input));
    } catch (e) {
      setError(asApiError(e).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Modal title={record ? `Edit ${record.type} record` : "Add DNS record"} subtitle={<span className="mono">{zone.name}</span>} onClose={onClose}
      footer={<><button className="btn" onClick={onClose}>Cancel</button><button className="btn primary" disabled={busy || !f.content.trim()} onClick={save}>{busy ? "Saving…" : record ? "Save" : "Add record"}</button></>}>
      <div className="form">
        <div className="field"><span>Type</span>
          <div className="segmented">{TYPES.map((t) => <button key={t} className={f.type === t ? "on" : ""} disabled={!!record && record.type !== t} onClick={() => set("type", t)}>{t}</button>)}</div>
          <small className="hint">{TYPE_HINT[f.type]}</small>
        </div>
        <label className="field"><span>Name</span>
          <div className="input-suffix"><input className="input mono" autoFocus placeholder="@ for the domain itself, or www, api, …" value={f.name} onChange={(e) => set("name", e.target.value)} /><span className="mono faint">.{zone.name}</span></div>
          <small className="hint mono">{full}</small>
        </label>
        <label className="field"><span>{f.type === "A" ? "IPv4 address" : f.type === "AAAA" ? "IPv6 address" : f.type === "CNAME" ? "Target name" : f.type === "MX" ? "Mail server" : "Text"}</span>
          {f.type === "TXT"
            ? <textarea className="input mono" rows={3} value={f.content} onChange={(e) => set("content", e.target.value.replace(/\n/g, ""))} />
            : <input className="input mono" placeholder={f.type === "A" ? "203.0.113.10" : f.type === "AAAA" ? "2001:db8::10" : "host.example.net"} value={f.content} onChange={(e) => set("content", e.target.value)} />}
        </label>
        {(f.type === "A" || f.type === "AAAA") && ips.length > 0 && (
          <div className="chips">
            <span className="faint">Your servers:</span>
            {ips.map((s) => <button key={s.label + s.address} className={`chip ${f.content === s.address ? "on" : ""}`} onClick={() => set("content", s.address)}>{s.label} · {s.address}</button>)}
          </div>
        )}
        <div className="row-fields">
          {f.type === "MX" && (
            <label className="field"><span>Priority</span><input className="input" type="number" min={0} max={65535} value={f.priority ?? 10} onChange={(e) => set("priority", Number(e.target.value))} /></label>
          )}
          <label className="field"><span>TTL</span>
            <select className="input" value={f.ttl} onChange={(e) => set("ttl", Number(e.target.value))}>{TTLS.map(([v, l]) => <option key={v} value={v}>{l}</option>)}</select>
          </label>
        </div>
        {proxiable && (
          <label className="check"><input type="checkbox" checked={f.proxied} onChange={(e) => set("proxied", e.target.checked)} />
            <span>Proxy through Cloudflare<br /><span className="faint" style={{ fontSize: 12 }}>Hides the server's address and adds Cloudflare's cache and protection. Leave off for SSH, mail and anything that isn't a website.</span></span>
          </label>
        )}
        {error && <div className="notice bad-text" style={{ margin: 0 }}>{error}</div>}
      </div>
    </Modal>
  );
}
