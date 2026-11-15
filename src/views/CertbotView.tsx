import { CheckCircle2, CircleDashed, FlaskConical, Globe, Lock, Plus, RefreshCw, RotateCw, Trash2, XCircle } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { confirm, Modal } from "../components/ui";
import { api, asApiError, Certificate, CertbotStatus, CertMethod, DnsCheck, ExecOutput, IssueRequest } from "../lib/api";
import { useRemote, withSafetyOverride } from "../lib/remote";
import { SessionState, useStore } from "../store";

function DaysLeft({ c }: { c: Certificate }) {
  if (c.daysLeft == null) return <span className="faint">—</span>;
  if (c.daysLeft < 0 || !c.valid) return <span className="badge bad">Expired</span>;
  const cls = c.daysLeft <= 7 ? "bad" : c.daysLeft <= 30 ? "warn" : "ok";
  return <span className={`badge ${cls}`}>{c.daysLeft} days left</span>;
}

const METHODS: { id: CertMethod; label: string; hint: string }[] = [
  { id: "nginx", label: "nginx", hint: "certbot uses the running nginx to prove the domain, then adds the certificate to the matching server blocks." },
  { id: "apache", label: "Apache", hint: "The same, for Apache." },
  { id: "webroot", label: "Folder", hint: "Places a check file in the folder your site already serves. Your web server config isn't touched; point it at the certificate yourself." },
  { id: "standalone", label: "Standalone", hint: "certbot starts its own web server on port 80, so nothing else may be using port 80 while it runs." },
];

/**
 * Does each name already point at this server? Let's Encrypt checks exactly
 * that, so a name that doesn't is the most common reason a request fails.
 */
function DnsReadiness({ sid, names }: { sid: string; names: string[] }) {
  const [res, setRes] = useState<{ addresses: string[]; checks: DnsCheck[] } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const key = names.filter((n) => n.includes(".") && !n.startsWith("*")).join(" ");
  const run = () => {
    if (!key) { setRes(null); return; }
    setBusy(true);
    api.dnsPointsHere(sid, key.split(" "))
      .then((r) => { setRes(r); setError(null); })
      .catch((e) => setError(asApiError(e).message))
      .finally(() => setBusy(false));
  };
  useEffect(() => {
    const t = setTimeout(run, 700);
    return () => clearTimeout(t);
  }, [sid, key]);
  if (!key) return null;
  const bad = res?.checks.filter((c) => !c.verified) ?? [];
  return (
    <div className={`notice dns-ready ${res && bad.length === 0 && res.checks.length > 0 ? "ok" : ""}`} style={{ margin: 0 }}>
      <div className="dns-ready-head">
        <b>Do the names point at this server?</b>
        <span className="spacer" />
        <button className="icon-btn" aria-label="Check DNS again" onClick={run}><RefreshCw size={13} className={busy ? "spin" : ""} /></button>
      </div>
      {error && <div className="bad-text">{error}</div>}
      {res && res.addresses.length === 0 && (
        <div className="faint">Orbital couldn't find a public address on this server (it may sit behind a NAT or load balancer), so it can't tell. Check the names on the Domains page.</div>
      )}
      {res && res.checks.map((c) => {
        const seen = [...new Set(c.answers.flatMap((a) => a.values))];
        return (
          <div key={c.name} className="dns-ready-row">
            {c.verified ? <CheckCircle2 size={13} className="ok-text" /> : seen.length ? <XCircle size={13} className="bad-text" /> : <CircleDashed size={13} className="faint" />}
            <span className="mono">{c.name}</span>
            <span className="faint">{c.verified ? `→ ${c.expected.filter((e) => seen.includes(e)).join(", ")}` : seen.length ? `points at ${seen.join(", ")}, not this server (${res.addresses.join(", ")})` : `no ${c.type} record yet`}</span>
          </div>
        );
      })}
      {res && bad.length > 0 && (
        <div className="dns-ready-foot">
          <span className="faint">Let's Encrypt will refuse these until they point at {bad[0].expected.join(" or ")}.</span>
          <button className="btn" onClick={() => useStore.getState().setGlobalView("domains")}><Globe size={13} /> Open Domains</button>
        </div>
      )}
    </div>
  );
}

function IssueDialog({ s, status, initialDomains, onClose, onDone }: { s: SessionState; status: CertbotStatus; initialDomains: string[]; onClose: () => void; onDone: (out: ExecOutput, title: string) => void }) {
  const sid = s.info.id;
  const has = (p: string) => status.plugins.includes(p);
  const [domains, setDomains] = useState(initialDomains.join(" "));
  const [method, setMethod] = useState<CertMethod>(has("nginx") ? "nginx" : has("apache") ? "apache" : "webroot");
  const [webroot, setWebroot] = useState("/var/www/html");
  const [email, setEmail] = useState("");
  const [redirect, setRedirect] = useState(true);
  const [staging, setStaging] = useState(false);
  const [agree, setAgree] = useState(false);
  const [busy, setBusy] = useState(false);
  const [suggest, setSuggest] = useState<string[]>([]);

  // Domains nginx serves without HTTPS are the likely candidates.
  useEffect(() => {
    if (!s.tools?.some((t) => t.id === "nginx")) return;
    api.nginxConfig(sid).then((cfg) => {
      const covered = new Set(status.certificates.flatMap((c) => c.domains));
      const names = cfg.sites.filter((x) => !x.ssl).flatMap((x) => x.serverNames).filter((n) => n.includes(".") && !n.startsWith("*") && !n.startsWith("~") && !covered.has(n));
      setSuggest([...new Set(names)]);
    }).catch(() => {});
  }, [sid]);

  const list = domains.split(/[\s,]+/).map((d) => d.trim()).filter(Boolean);
  const add = (d: string) => !list.includes(d) && setDomains([...list, d].join(" "));
  async function issue() {
    setBusy(true);
    const req: IssueRequest = { domains: list, email: email.trim() || null, method, webroot: method === "webroot" ? webroot : null, redirect, staging, agreeTos: agree };
    try {
      const out = await api.certbotIssue(sid, req);
      onDone(out, out.exitCode === 0 ? `Certificate issued for ${list[0]}` : "Let's Encrypt didn't issue the certificate");
      if (out.exitCode === 0) onClose();
    } catch (e) {
      useStore.getState().toast({ kind: "error", title: "Couldn't request the certificate", body: asApiError(e).message });
    } finally {
      setBusy(false);
    }
  }
  const m = METHODS.find((x) => x.id === method)!;
  return (
    <Modal title="New HTTPS certificate" wide subtitle="Free certificates from Let's Encrypt, valid for 90 days and renewed automatically." onClose={onClose}
      footer={<><button className="btn" onClick={onClose}>Cancel</button><button className="btn primary" disabled={busy || !agree || list.length === 0} onClick={issue}>{busy ? "Requesting… this can take a minute" : "Request certificate"}</button></>}>
      <div className="form">
        <div className="notice" style={{ margin: 0 }}>
          Before you start: each domain's DNS record (A or AAAA) must already point at this server, and port 80 must be reachable from the internet. Let's Encrypt connects to the domain to check it.
        </div>
        <label className="field"><span>Domain names</span>
          <input className="input mono" autoFocus placeholder="example.com www.example.com" value={domains} onChange={(e) => setDomains(e.target.value)} />
          <small className="hint">One certificate can cover several names, separated by spaces. Wildcards (*.example.com) need a DNS challenge and aren't supported yet.</small>
        </label>
        {suggest.length > 0 && (
          <div className="chips">
            <span className="faint">nginx sites without HTTPS:</span>
            {suggest.map((d) => <button key={d} className={`chip ${list.includes(d) ? "on" : ""}`} onClick={() => add(d)}>{d}</button>)}
          </div>
        )}
        <DnsReadiness sid={sid} names={list} />
        <div className="field"><span>How to prove you own the domain</span>
          <div className="segmented">
            {METHODS.map((x) => (
              <button key={x.id} className={method === x.id ? "on" : ""} disabled={x.id !== "webroot" && !has(x.id)} title={x.id !== "webroot" && !has(x.id) ? `The certbot ${x.id} plugin isn't installed` : ""} onClick={() => setMethod(x.id)}>{x.label}</button>
            ))}
          </div>
          <small className="hint">{m.hint}</small>
        </div>
        {method === "webroot" && (
          <label className="field"><span>Folder the site serves</span><input className="input mono" value={webroot} onChange={(e) => setWebroot(e.target.value)} /></label>
        )}
        <label className="field"><span>E-mail for expiry warnings (optional)</span><input className="input" placeholder="you@example.com" value={email} onChange={(e) => setEmail(e.target.value)} /></label>
        {(method === "nginx" || method === "apache") && (
          <label className="check"><input type="checkbox" checked={redirect} onChange={(e) => setRedirect(e.target.checked)} /><span>Send visitors from http:// to https:// automatically</span></label>
        )}
        <label className="check"><input type="checkbox" checked={staging} onChange={(e) => setStaging(e.target.checked)} /><span>Test run against Let's Encrypt's staging servers (the certificate won't be trusted by browsers, but there are no rate limits)</span></label>
        <label className="check"><input type="checkbox" checked={agree} onChange={(e) => setAgree(e.target.checked)} /><span>I accept the Let's Encrypt Subscriber Agreement (letsencrypt.org/repository)</span></label>
      </div>
    </Modal>
  );
}

/** Let's Encrypt certificates on this server: expiry, renewal, and new ones for your domains. */
export function CertbotView({ s }: { s: SessionState }) {
  const { toast, patch } = useStore();
  const sid = s.info.id;
  const { data: st, error, loading, reload } = useRemote(() => api.certbotStatus(sid), [sid]);
  const [sel, setSel] = useState<string | null>(null);
  const [issuing, setIssuing] = useState<string[] | null>(null);
  const [output, setOutput] = useState<{ title: string; out: ExecOutput } | null>(null);
  const [busy, setBusy] = useState(false);

  // "Get HTTPS certificate" from the nginx tab lands here with its domains.
  useEffect(() => {
    if (st && s.certDomains) {
      setIssuing(s.certDomains);
      patch(sid, { certDomains: null });
    }
  }, [st, s.certDomains]);

  const sorted = useMemo(() => [...(st?.certificates ?? [])].sort((a, b) => (a.daysLeft ?? 0) - (b.daysLeft ?? 0)), [st]);
  const cur = sorted.find((c) => c.name === sel) ?? sorted[0] ?? null;

  const show = (out: ExecOutput, title: string) => {
    setOutput({ title, out });
    reload();
  };

  async function renew(name: string | null, mode: "dryRun" | "force") {
    if (mode === "force") {
      const ok = await confirm({ title: `Renew ${name} now?`, body: "Let's Encrypt allows only 5 renewals of the same certificate per week. Certificates renew automatically 30 days before they expire, so this is rarely needed.", confirmLabel: "Renew now" });
      if (!ok) return;
    }
    setBusy(true);
    try {
      const out = await api.certbotRenew(sid, name, mode);
      const ok = out.exitCode === 0;
      show(out, mode === "dryRun" ? (ok ? "Renewal test passed" : "Renewal test failed") : ok ? `${name} renewed` : "Renewal failed");
    } catch (e) {
      toast({ kind: "error", title: "Couldn't run certbot", body: asApiError(e).message });
    } finally {
      setBusy(false);
    }
  }

  async function remove(c: Certificate) {
    const ok = await confirm({ title: `Delete the certificate for ${c.name}?`, body: "The certificate and its private key are removed from this server. Sites using it stop working over HTTPS.", confirmLabel: "Delete certificate", danger: true, requireText: c.name });
    if (!ok) return;
    setBusy(true);
    try {
      const out = await withSafetyOverride((force) => api.certbotDelete(sid, c.name, force), "a site still uses it");
      if (out) show(out, out.exitCode === 0 ? `${c.name} deleted` : "Delete failed");
    } catch (e) {
      toast({ kind: "error", title: "Couldn't delete the certificate", body: asApiError(e).message });
    } finally {
      setBusy(false);
    }
  }

  const ar = st?.autoRenew;
  return (
    <div className="view">
      <div className="toolbar">
        {st && (ar ? (ar.active
          ? <span className="badge ok" title={ar.via}>Renews automatically{ar.nextRun ? ` · next check ${ar.nextRun}` : ""}</span>
          : <span className="badge bad" title={ar.via}>Automatic renewal is off ({ar.via})</span>)
          : <span className="badge bad">No automatic renewal set up</span>)}
        {st?.version && <span className="faint">certbot {st.version}</span>}
        <span className="spacer" />
        {st && <button className="btn" disabled={busy || st.certificates.length === 0} title="Rehearse renewing every certificate against the test servers. Nothing changes." onClick={() => renew(null, "dryRun")}><FlaskConical size={13} /> Test renewal</button>}
        {st && <button className="btn primary" disabled={busy} onClick={() => setIssuing([])}><Plus size={13} /> New certificate</button>}
        <button className="icon-btn" aria-label="Refresh" onClick={reload}><RefreshCw size={14} className={loading ? "spin" : ""} /></button>
      </div>
      {error && <div className="empty"><div><h3>Couldn't read certificates</h3><p>{error.message}</p><button className="btn" onClick={reload}>Try again</button></div></div>}
      {st?.error && <div className="notice">{st.error}</div>}
      {st && !st.error && st.certificates.length === 0 && (
        <div className="empty"><div>
          <Lock size={28} className="faint" />
          <h3>No certificates yet</h3>
          <p>Get a free HTTPS certificate from Let's Encrypt for a domain that points at this server.</p>
          <button className="btn primary" onClick={() => setIssuing([])}><Plus size={13} /> New certificate</button>
        </div></div>
      )}
      {st && sorted.length > 0 && (
        <div className="split">
          <div className="scroll">
            <table className="table">
              <thead><tr><th>Domains</th><th>Expires</th><th>Status</th><th>Key</th></tr></thead>
              <tbody>
                {sorted.map((c) => (
                  <tr key={c.name} className={`click ${cur?.name === c.name ? "sel" : ""}`} onClick={() => setSel(c.name)}>
                    <td className="mono"><div className="cell-clip" style={{ maxWidth: 300 }} title={c.domains.join(" ")}>{c.domains.join(", ")}</div></td>
                    <td className="muted nowrap">{c.expires ? new Date(c.expires).toLocaleDateString() : "—"}</td>
                    <td><DaysLeft c={c} /></td>
                    <td className="faint">{c.keyType ?? "—"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <aside className="drawer">
            {cur && (
              <>
                <div className="drawer-head"><h3 className="mono" style={{ wordBreak: "break-all" }}>{cur.name}</h3><div className="muted">{cur.status || "—"}</div></div>
                <div className="actions">
                  <button className="btn" disabled={busy} onClick={() => renew(cur.name, "dryRun")}><FlaskConical size={13} /> Test renewal</button>
                  <button className="btn" disabled={busy} onClick={() => renew(cur.name, "force")}><RotateCw size={13} /> Renew now</button>
                  <button className="btn danger" disabled={busy} onClick={() => remove(cur)}><Trash2 size={13} /> Delete</button>
                </div>
                <dl className="kv">
                  <dt>Covers</dt><dd className="mono">{cur.domains.join("\n")}</dd>
                  <dt>Expires</dt><dd>{cur.expires ? new Date(cur.expires).toLocaleString() : "—"}</dd>
                  <dt>Key type</dt><dd>{cur.keyType ?? "—"}</dd>
                  <dt>Certificate</dt><dd className="mono">{cur.certificatePath ?? "—"}</dd>
                  <dt>Private key</dt><dd className="mono">{cur.privateKeyPath ?? "—"}</dd>
                </dl>
                <p className="footnote">Point a web server at the certificate file (fullchain.pem) and the private key. The private key never leaves the server.</p>
              </>
            )}
          </aside>
        </div>
      )}
      {issuing && st && <IssueDialog s={s} status={st} initialDomains={issuing} onClose={() => setIssuing(null)} onDone={show} />}
      {output && (
        <Modal title={output.title} wide onClose={() => setOutput(null)} footer={<button className="btn primary" onClick={() => setOutput(null)}>Close</button>}>
          <pre className="output" style={{ maxHeight: 420 }}>{[output.out.stdout, output.out.stderr].filter((x) => x.trim()).join("\n") || "certbot printed nothing."}</pre>
        </Modal>
      )}
    </div>
  );
}
