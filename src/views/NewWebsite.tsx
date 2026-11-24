import { Check, CheckCircle2, Cloud, Copy, FileText, Globe, Lock, Server } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { Modal } from "../components/ui";
import { api, asApiError, CertbotStatus, DnsAccount, DnsRecord, DnsZone, ExecOutput, SiteKind, WebsiteInput, WebsitePlan, WebsiteReport } from "../lib/api";
import { runJob, useRunningJob } from "../lib/jobs";
import { BrokenSitesFix } from "../components/BrokenSites";
import { JobProgress } from "../components/JobProgress";
import { useStore } from "../store";
import { DnsReadiness } from "./CertbotView";
import { CheckPanel } from "./DomainsView";
import { SiteTestPanel } from "../components/SiteTest";

const STEPS = ["Domain", "DNS", "Site", "HTTPS"] as const;
type Step = 0 | 1 | 2 | 3;

/** Same rule as the engine's (it checks again): letters, digits, dashes, at least one dot. */
const DOMAIN_RE = /^(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z][a-z0-9-]{0,61}[a-z0-9]$/;
const normDomain = (d: string) => d.trim().toLowerCase().replace(/\.$/, "").replace(/^https?:\/\//, "").replace(/\/.*$/, "");
const isIpv4 = (a: string) => /^\d{1,3}(\.\d{1,3}){3}$/.test(a);

type Covering = { account: DnsAccount; zone: DnsZone };
/** One A record the site needs, and what the zone has for that name today. */
type Want = { name: string; existing: DnsRecord | null };

/** The Cloudflare zone a domain belongs to (the longest one that matches), if any account has it. */
async function findZone(domain: string): Promise<Covering | null> {
  const accounts = await api.dnsAccounts();
  const all = await Promise.all(accounts.map((account) => api.dnsZones(account.id).then((zones) => zones.map((zone) => ({ account, zone })), () => [] as Covering[])));
  const hits = all.flat().filter(({ zone }) => domain === zone.name || domain.endsWith(`.${zone.name}`));
  return hits.sort((a, b) => b.zone.name.length - a.zone.name.length)[0] ?? null;
}

function CopyButton({ text }: { text: string }) {
  const [done, setDone] = useState(false);
  return (
    <button className="icon-btn" aria-label="Copy" title="Copy" onClick={() => { void navigator.clipboard?.writeText(text); setDone(true); setTimeout(() => setDone(false), 1200); }}>
      {done ? <Check size={13} className="ok-text" /> : <Copy size={13} />}
    </button>
  );
}

/**
 * New website: point a domain at this server, give nginx a server block for
 * it, and get it a certificate. Each step uses the same engine calls as the
 * Domains page, safe save and the certificates tab.
 */
export function NewWebsiteDialog({ sid }: { sid: string }) {
  const { toast, openFile, setNewWebsite } = useStore();
  const s = useStore((x) => x.sessions[sid]);
  const close = () => setNewWebsite(null);
  // The connection can drop while this is open (the Mac slept, the network changed):
  // reconnect from here and carry on with everything filled in so far.
  const [reconnecting, setReconnecting] = useState(false);
  async function reconnectHere() {
    setReconnecting(true);
    try {
      if (await useStore.getState().reconnect(sid)) {
        setPlanError(null);
        setCertbotError(null);
      }
    } finally {
      setReconnecting(false);
    }
  }
  const [step, setStep] = useState<Step>(0);
  const [done, setDone] = useState<Set<Step>>(new Set());
  const finish = (n: Step) => setDone((d) => new Set(d).add(n));

  // ----- step 1: domain and DNS record -----
  const [domainText, setDomainText] = useState("");
  const [wwwPref, setWww] = useState<boolean | null>(null);
  const domain = normDomain(domainText);
  // www. by default for example.com, not for blog.example.com.
  const www = wwwPref ?? domain.split(".").length === 2;
  const domainOk = DOMAIN_RE.test(domain);
  const names = useMemo(() => (domainOk ? (www && !domain.startsWith("www.") ? [domain, `www.${domain}`] : [domain]) : []), [domain, domainOk, www]);
  const [addresses, setAddresses] = useState<string[] | null>(null);
  const [address, setAddress] = useState("");
  const [cover, setCover] = useState<Covering | null | undefined>(undefined);
  const [wants, setWants] = useState<Want[] | null>(null);
  const [looking, setLooking] = useState(false);
  const [writing, setWriting] = useState(false);

  useEffect(() => {
    api.serverPublicAddresses(sid).then((a) => {
      setAddresses(a);
      setAddress(a.find(isIpv4) ?? "");
    }, () => setAddresses([]));
  }, [sid]);

  // Look the domain up in the connected DNS accounts, a moment after typing stops.
  useEffect(() => {
    setCover(undefined);
    setWants(null);
    if (!domainOk) return;
    let live = true;
    const t = setTimeout(async () => {
      setLooking(true);
      try {
        const c = await findZone(domain);
        if (!live) return;
        setCover(c);
        if (c) {
          const recs = await api.dnsRecords(c.account.id, c.zone.id);
          if (!live) return;
          setWants(names.map((name) => ({ name, existing: recs.find((r) => r.name === name && ["A", "AAAA", "CNAME"].includes(r.type)) ?? null })));
        }
      } catch {
        if (live) setCover(null);
      } finally {
        if (live) setLooking(false);
      }
    }, 600);
    return () => { live = false; clearTimeout(t); };
  }, [domain, domainOk, names.join()]);

  const pointsHere = (w: Want) => w.existing?.type === "A" && w.existing.content === address && !w.existing.proxied;
  const todo = (wants ?? []).filter((w) => !pointsHere(w));
  const blockedBy = todo.filter((w) => w.existing && w.existing.type !== "A");

  async function writeRecords() {
    if (!cover || !wants) return;
    setWriting(true);
    try {
      const out: Want[] = [];
      for (const w of wants) {
        if (pointsHere(w)) { out.push(w); continue; }
        const input = { type: "A" as const, name: w.name, content: address, ttl: 1, proxied: false, priority: null };
        const rec = w.existing?.type === "A"
          ? await api.updateDnsRecord(cover.account.id, cover.zone.id, cover.zone.name, w.existing.id, input)
          : await api.createDnsRecord(cover.account.id, cover.zone.id, cover.zone.name, input);
        out.push({ name: w.name, existing: rec });
      }
      setWants(out);
      toast({ kind: "success", title: out.length > 1 ? "DNS records saved" : "DNS record saved", body: `${names.join(", ")} → ${address}` });
      finish(0);
      setStep(1);
    } catch (e) {
      toast({ kind: "error", title: "Couldn't save the DNS record", body: asApiError(e).message });
    } finally {
      setWriting(false);
    }
  }

  // ----- step 2: wait for DNS -----
  const [live, setLive] = useState<Record<string, boolean>>({});
  const allLive = names.length > 0 && names.every((n) => live[n]);
  useEffect(() => { if (allLive) finish(1); }, [allLive]);

  // ----- step 3: nginx site -----
  const [kind, setKind] = useState<SiteKind["kind"]>("static");
  const [root, setRoot] = useState("");
  const [port, setPort] = useState("3000");
  const [plan, setPlan] = useState<WebsitePlan | null>(null);
  const [planRev, setPlanRev] = useState(0);
  const [planError, setPlanError] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [report, setReport] = useState<WebsiteReport | null>(null);
  const rootValue = root || `/var/www/${domain}`;
  const input: WebsiteInput = { domain, includeWww: www, site: kind === "static" ? { kind: "static", root: rootValue } : { kind: "proxy", port: Number(port) || 0 } };
  const inputKey = JSON.stringify(input);

  useEffect(() => {
    if (step !== 2 || report) return;
    let alive = true;
    const t = setTimeout(() => {
      api.websitePlan(sid, input).then((p) => { if (alive) { setPlan(p); setPlanError(null); } }, (e) => { if (alive) { setPlan(null); setPlanError(asApiError(e).message); } });
    }, 400);
    return () => { alive = false; clearTimeout(t); };
  }, [step, inputKey, report, sid, planRev]);

  const job = useRunningJob(`website:${sid}`);
  async function createSite() {
    setCreating(true);
    try {
      const rep = await runJob((p) => api.createWebsite(sid, input, p), { key: `website:${sid}`, title: `New website ${domain}`, sid, done: (r) => (r.reloaded ? `${domain} is set up in nginx` : { failed: "Saved, but nginx didn't reload" }) });
      setReport(rep);
      finish(2);
      useStore.setState((x) => ({ websiteRev: x.websiteRev + 1 }));
      if (rep.reloaded) toast({ kind: "success", title: `${domain} is set up in nginx`, body: `nginx -t passed and nginx reloaded.` });
      else toast({ kind: "error", title: "Saved, but nginx didn't reload", body: rep.reloadOutput });
    } catch (e) {
      const err = asApiError(e);
      setPlanError(err.kind === "ValidationFailed" ? `nginx -t failed, so nothing was kept:\n${err.data.output}` : err.message);
    } finally {
      setCreating(false);
    }
  }

  // ----- step 4: certificate -----
  const [certbot, setCertbot] = useState<CertbotStatus | null>(null);
  const [certbotError, setCertbotError] = useState<string | null>(null);
  const [email, setEmail] = useState("");
  const [redirect, setRedirect] = useState(true);
  const [staging, setStaging] = useState(false);
  const [agree, setAgree] = useState(false);
  const [issuing, setIssuing] = useState(false);
  const [certOut, setCertOut] = useState<ExecOutput | null>(null);
  const hasCertbot = !!s?.tools?.some((t) => t.id === "certbot");
  useEffect(() => {
    if (step !== 3 || certbot || !hasCertbot) return;
    api.certbotStatus(sid).then(setCertbot, (e) => setCertbotError(asApiError(e).message));
  }, [step, hasCertbot, sid]);

  async function issue() {
    setIssuing(true);
    try {
      const out = await runJob(
        (p) => api.certbotIssue(sid, { domains: names, email: email.trim() || null, method: "nginx", webroot: null, redirect, staging, agreeTos: agree }, p),
        { key: `website:${sid}`, title: `Certificate for ${domain}`, sid, done: (o) => (o.exitCode === 0 ? `https://${domain} is live` : { failed: "Let's Encrypt didn't issue the certificate" }) },
      );
      setCertOut(out);
      if (out.exitCode === 0) {
        finish(3);
        useStore.setState((x) => ({ websiteRev: x.websiteRev + 1 }));
        toast({ kind: "success", title: `https://${domain} is live`, body: staging ? "Test certificate from the staging servers; browsers won't trust it." : undefined });
      }
    } catch {
      // The job reports the error.
    } finally {
      setIssuing(false);
    }
  }

  // ----- layout -----
  const ipOk = isIpv4(address);
  const footer = (() => {
    const back = step > 0 && <button className="btn" onClick={() => setStep((step - 1) as Step)}>Back</button>;
    if (step === 0) {
      return <>
        <span className="spacer" />
        <button className="btn" onClick={close}>Cancel</button>
        {cover && wants && todo.length > 0
          ? <>
              <button className="btn" disabled={!domainOk} onClick={() => setStep(1)} title="Leave DNS as it is">Skip</button>
              <button className="btn primary" disabled={!ipOk || writing || blockedBy.length > 0} onClick={writeRecords}><Cloud size={13} /> {writing ? "Saving…" : `Save ${todo.length > 1 ? `${todo.length} records` : "record"} in Cloudflare`}</button>
            </>
          : <button className="btn primary" disabled={!domainOk || looking} onClick={() => { finish(0); setStep(1); }}>Next</button>}
      </>;
    }
    if (step === 1) {
      return <>{back}<span className="spacer" />
        {!allLive && <button className="btn" onClick={() => setStep(2)} title="Carry on; Let's Encrypt will need the records live before step 4">Continue without waiting</button>}
        <button className="btn primary" disabled={!allLive} onClick={() => setStep(2)}>Next</button>
      </>;
    }
    if (step === 2) {
      return <>{!report && back}<span className="spacer" />
        {report
          ? <button className="btn primary" onClick={() => setStep(3)}>Next</button>
          : <button className="btn primary" disabled={creating || !plan || plan.blockers.length > 0} onClick={createSite}>{creating ? "Creating…" : "Create site"}</button>}
      </>;
    }
    return <><span className="spacer" />
      {done.has(3) || !hasCertbot || !certbot?.plugins.includes("nginx")
        ? <button className="btn primary" onClick={close}>Done</button>
        : <><button className="btn" onClick={close}>Skip HTTPS</button><button className="btn primary" disabled={issuing || !agree} onClick={issue}>{issuing ? "Requesting…" : "Request certificate"}</button></>}
    </>;
  })();

  return (
    <Modal title={domainOk && step > 0 ? `New website: ${domain}` : "New website"} wide subtitle={`On ${s?.profile.name ?? "this server"}, served by nginx.`}
      onClose={close} keepOnOutsideClick footer={footer}>
      <ol className="wizard-steps">
        {STEPS.map((label, i) => (
          <li key={label} className={`${i === step ? "on" : ""} ${done.has(i as Step) ? "done" : ""}`}>
            <span className="wizard-num">{done.has(i as Step) ? <Check size={11} /> : i + 1}</span>{label}
          </li>
        ))}
      </ol>
      {job && <div style={{ marginBottom: 12 }}><JobProgress job={job} /></div>}
      {s?.lost && (
        <div className="alert lost-here">
          <span>The connection to {s.profile.name} was lost (the Mac slept, the network changed or the server restarted). What you filled in is kept.</span>
          <button className="btn primary" disabled={reconnecting} onClick={reconnectHere}>{reconnecting ? "Reconnecting…" : "Reconnect"}</button>
        </div>
      )}

      {step === 0 && (
        <div className="form">
          <label className="field"><span>Domain name</span>
            <input className="input mono" autoFocus placeholder="example.com" value={domainText} onChange={(e) => setDomainText(e.target.value)} />
            {domainText && !domainOk && <small className="hint bad-text">Type a domain such as example.com or shop.example.com, without http://.</small>}
          </label>
          {!domain.startsWith("www.") && (
            <label className="check"><input type="checkbox" checked={www} onChange={(e) => setWww(e.target.checked)} /><span>Also answer for <span className="mono">www.{domain || "example.com"}</span></span></label>
          )}
          <label className="field"><span>This server's public address</span>
            <input className="input mono" style={{ maxWidth: 260 }} placeholder="203.0.113.10" value={address} onChange={(e) => setAddress(e.target.value.trim())} />
            <small className="hint">
              {addresses === null ? "Looking…" : addresses.length
                ? `Found on the server: ${addresses.join(", ")}. The record uses the IPv4 address.`
                : "Orbital couldn't find a public address on the server (it may sit behind a NAT or load balancer). Type the address visitors reach it on."}
            </small>
          </label>

          {domainOk && looking && <div className="notice" style={{ margin: 0 }}>Looking for {domain} in your DNS accounts…</div>}
          {domainOk && !looking && cover && wants && (
            <div className="notice" style={{ margin: 0 }}>
              <div className="dns-ready-head"><Cloud size={14} /> <b>{cover.zone.name} is in your Cloudflare account “{cover.account.name}”</b></div>
              {cover.zone.status !== "active" && <div className="warn-text">The zone is {cover.zone.status}: the domain's registrar must use {cover.zone.nameServers.join(" and ")} before these records count.</div>}
              <table className="table dns-answers" style={{ marginTop: 6 }}>
                <tbody>
                  {wants.map((w) => (
                    <tr key={w.name}>
                      <td className="mono nowrap">A {w.name}</td>
                      <td className="mono">
                        {pointsHere(w) ? <span className="ok-text"><CheckCircle2 size={12} /> already points at {address}</span>
                          : !w.existing ? <span>will be created → {address || "?"}</span>
                          : w.existing.type === "A" && w.existing.content === address ? <span className="warn-text">is proxied through Cloudflare; will be set to DNS only</span>
                          : w.existing.type === "A" ? <span className="warn-text">points at {w.existing.content}{w.existing.proxied ? " (proxied)" : ""}; will be changed to {address || "?"}</span>
                          : <span className="bad-text">is a {w.existing.type} record ({w.existing.content}); change or delete it on the Domains page first</span>}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
              <small className="faint">Not proxied through Cloudflare, so Let's Encrypt can reach the server directly.</small>
            </div>
          )}
          {domainOk && !looking && cover === null && (
            <div className="notice" style={{ margin: 0 }}>
              <div className="dns-ready-head"><Globe size={14} /> <b>Add {names.length > 1 ? "these records" : "this record"} where {domain}'s DNS is managed</b></div>
              <table className="table dns-answers" style={{ marginTop: 6 }}>
                <thead><tr><th>Type</th><th>Name</th><th>Value</th><th /></tr></thead>
                <tbody>
                  {names.map((n) => (
                    <tr key={n}>
                      <td className="mono">A</td>
                      <td className="mono">{n}</td>
                      <td className="mono">{address || "?"}</td>
                      <td style={{ width: 30 }}>{address && <CopyButton text={address} />}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
              <small className="faint">None of your connected DNS accounts has this domain. Connect Cloudflare on the Domains page to have Orbital add records for you.</small>
            </div>
          )}
        </div>
      )}

      {step === 1 && (
        <div>
          <p className="muted" style={{ margin: 0 }}>Orbital asks public resolvers every 10 seconds, the same answer browsers and Let's Encrypt get.</p>
          {!ipOk && <div className="notice bad-text" style={{ margin: "12px 0 0" }}>Go back and enter the server's IPv4 address to check against.</div>}
          {ipOk && names.map((n) => (
            <CheckPanel key={`${n}|${address}`} name={n} type="A" expected={[address]} keepChecking onResult={(v) => setLive((m) => (m[n] === v ? m : { ...m, [n]: v }))} />
          ))}
        </div>
      )}

      {step === 2 && !report && (
        <div className="form">
          <div className="field"><span>What the site serves</span>
            <div className="segmented" style={{ alignSelf: "flex-start" }}>
              <button className={kind === "static" ? "on" : ""} onClick={() => setKind("static")}>Files from a folder</button>
              <button className={kind === "proxy" ? "on" : ""} onClick={() => setKind("proxy")}>An app on this server</button>
            </div>
          </div>
          {kind === "static"
            ? <label className="field"><span>Folder</span>
                <input className="input mono" value={rootValue} onChange={(e) => setRoot(e.target.value)} />
                <small className="hint">{plan?.root && !plan.rootExists ? "Doesn't exist yet: it's created with a placeholder index.html." : "nginx serves the files in this folder."}</small>
              </label>
            : <label className="field"><span>Port the app listens on</span>
                <input className="input mono" style={{ maxWidth: 120 }} inputMode="numeric" value={port} onChange={(e) => setPort(e.target.value.replace(/\D/g, "").slice(0, 5))} />
                <small className="hint">nginx forwards requests to http://127.0.0.1:{port || "…"}, with WebSocket support.</small>
              </label>}
          {planError && <div className="alert" style={{ whiteSpace: "pre-wrap" }}>{planError}</div>}
          {plan && plan.blockers.length > 0 && (
            <div className="alert"><b>Can't create this site yet:</b><ul className="steps" style={{ marginTop: 6 }}>{plan.blockers.map((b) => <li key={b} style={{ whiteSpace: "pre-wrap" }}>{b}</li>)}</ul>
              <BrokenSitesFix sid={sid} sites={plan.brokenSites ?? []} onFixed={() => setPlanRev((n) => n + 1)} />
            </div>
          )}
          {plan && (
            <div className="field"><span>
              {plan.enabledLink ? <>Writes <span className="mono">{plan.file}</span> and enables it in <span className="mono">sites-enabled/</span></> : <>Writes <span className="mono">{plan.file}</span></>}
              {"; "}nginx -t checks it, and it's removed again if the test fails.
            </span>
              <pre className="output">{plan.content}</pre>
            </div>
          )}
        </div>
      )}
      {step === 2 && report && (
        <div className="form">
          <div className="notice" style={{ margin: 0 }}>
            <div className="dns-ready-head"><CheckCircle2 size={14} className="ok-text" /> <b>{domain} is set up in nginx</b></div>
            <dl className="kv" style={{ padding: "8px 0 0" }}>
              <dt>Server block</dt><dd className="mono">{report.plan.file}</dd>
              {report.plan.enabledLink && <><dt>Enabled as</dt><dd className="mono">{report.plan.enabledLink}</dd></>}
              {report.plan.root && <><dt>Files from</dt><dd className="mono">{report.plan.root}{report.rootCreated ? " (new, with a placeholder page)" : ""}</dd></>}
              <dt>nginx -t</dt><dd className="mono">{report.write.validatorOutput?.trim().split("\n").pop() || "passed"}</dd>
              <dt>Reload</dt><dd className={report.reloaded ? "" : "bad-text"}>{report.reloaded ? "done" : report.reloadOutput || "failed"}</dd>
            </dl>
          </div>
          <SiteTestPanel sid={sid} domain={domain} https={false} />
          <div><button className="btn" onClick={() => { close(); void openFile(sid, report.plan.file); }}><FileText size={13} /> Open in the editor</button></div>
        </div>
      )}

      {step === 3 && (
        <div className="form">
          {!hasCertbot && (
            <div className="notice" style={{ margin: 0 }}>
              certbot isn't installed on this server. Install it with <span className="mono">apt install certbot python3-certbot-nginx</span> (or your system's equivalent), then use <b>Get HTTPS certificate</b> on the site in the nginx tab.
            </div>
          )}
          {hasCertbot && !certbot && !certbotError && <div className="notice" style={{ margin: 0 }}>Reading certbot's status…</div>}
          {certbotError && <div className="alert">{certbotError}</div>}
          {certbot && !certbot.plugins.includes("nginx") && (
            <div className="notice" style={{ margin: 0 }}>
              certbot's nginx plugin isn't installed (<span className="mono">python3-certbot-nginx</span>). Install it, or request the certificate from the HTTPS certificates tab with the Folder method.
            </div>
          )}
          {certbot?.plugins.includes("nginx") && !done.has(3) && (
            <>
              <p className="muted" style={{ margin: 0 }}>A free Let's Encrypt certificate for <span className="mono">{names.join(" and ")}</span>. certbot proves the domain through nginx and adds HTTPS to the new server block; it renews automatically.</p>
              <DnsReadiness sid={sid} names={names} />
              <label className="field"><span>E-mail for expiry warnings (optional)</span><input className="input" placeholder="you@example.com" value={email} onChange={(e) => setEmail(e.target.value)} /></label>
              <label className="check"><input type="checkbox" checked={redirect} onChange={(e) => setRedirect(e.target.checked)} /><span>Send visitors from http:// to https:// automatically</span></label>
              <label className="check"><input type="checkbox" checked={staging} onChange={(e) => setStaging(e.target.checked)} /><span>Test run against Let's Encrypt's staging servers (not trusted by browsers, no rate limits)</span></label>
              <label className="check"><input type="checkbox" checked={agree} onChange={(e) => setAgree(e.target.checked)} /><span>I accept the Let's Encrypt Subscriber Agreement (letsencrypt.org/repository)</span></label>
            </>
          )}
          {done.has(3) && (
            <div className="notice" style={{ margin: 0 }}>
              <div className="dns-ready-head"><Lock size={14} className="ok-text" /> <b>https://{domain} has its certificate</b></div>
              <span className="faint">Renewed automatically before it expires. It's listed in the HTTPS certificates tab.</span>
            </div>
          )}
          {done.has(3) && <SiteTestPanel sid={sid} domain={domain} https />}
          {certOut && <pre className={`output ${certOut.exitCode === 0 ? "" : "bad-text"}`}>{(certOut.stdout + certOut.stderr).trim()}</pre>}
        </div>
      )}
      {step > 0 && <small className="faint"><Server size={11} style={{ verticalAlign: -1 }} /> Every change is in the Activity log.</small>}
    </Modal>
  );
}
