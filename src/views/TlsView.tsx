import { CheckCircle2, Lock, Search, ShieldAlert, XCircle } from "lucide-react";
import { useState } from "react";
import { api, asApiError, TlsReport } from "../lib/api";

const RECENT = "orbital.tlsRecent";
function loadRecent(): string[] {
  try {
    return JSON.parse(localStorage.getItem(RECENT) ?? "[]");
  } catch {
    return [];
  }
}

/** "example.com", "example.com:8443", "https://example.com/x", "[2001:db8::1]:443" → host and port. */
export function parseTarget(input: string): { host: string; port: number } | null {
  let t = input.trim();
  if (!t) return null;
  let port = 443;
  const scheme = t.match(/^([a-z]+):\/\//i);
  if (scheme) {
    const known: Record<string, number> = { https: 443, smtps: 465, imaps: 993, pop3s: 995, ldaps: 636, ftps: 990 };
    port = known[scheme[1].toLowerCase()] ?? 443;
    t = t.slice(scheme[0].length);
  }
  t = t.split(/[/?#]/)[0];
  const v6 = t.match(/^\[([^\]]+)\](?::(\d+))?$/);
  if (v6) return { host: `[${v6[1]}]`, port: v6[2] ? Number(v6[2]) : port };
  const m = t.match(/^([^:]+)(?::(\d+))?$/);
  if (!m) return null;
  const p = m[2] ? Number(m[2]) : port;
  if (!(p > 0 && p < 65536)) return null;
  return { host: m[1].toLowerCase(), port: p };
}

const day = (unix: number) => new Date(unix * 1000).toLocaleDateString(undefined, { dateStyle: "medium" });

/** Check any TLS server's certificate from this computer: chain, expiry, names, protocol. */
export function TlsView() {
  const [input, setInput] = useState("");
  const [r, setR] = useState<TlsReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [recent, setRecent] = useState<string[]>(loadRecent);

  async function check(target = input) {
    const t = parseTarget(target);
    if (!t) return setError("Write a host name, optionally with a port: example.com or mail.example.com:993");
    setBusy(true);
    setError(null);
    try {
      setR(await api.tlsInspect(t.host, t.port));
      const label = t.port === 443 ? t.host : `${t.host}:${t.port}`;
      const next = [label, ...recent.filter((x) => x !== label)].slice(0, 12);
      setRecent(next);
      try { localStorage.setItem(RECENT, JSON.stringify(next)); } catch { /* private mode */ }
    } catch (e) {
      setR(null);
      setError(asApiError(e).message);
    } finally {
      setBusy(false);
    }
  }

  const leaf = r?.chain[0];
  return (
    <div className="view">
      <div className="scroll">
        <div className="settings" style={{ maxWidth: 900 }}>
          <div className="setting check-form">
            <label className="field grow"><span>Host (and port)</span>
              <input className="input mono" autoFocus placeholder="example.com, mail.example.com:993, https://example.com/" value={input} onChange={(e) => setInput(e.target.value)} onKeyDown={(e) => e.key === "Enter" && void check()} />
            </label>
            <button className="btn primary" disabled={busy || !input.trim()} onClick={() => void check()}><Search size={13} /> {busy ? "Checking…" : "Check"}</button>
          </div>
          {recent.length > 0 && (
            <div className="chips" style={{ margin: "6px 0 12px" }}>
              <span className="faint">Recent:</span>
              {recent.map((x) => <button key={x} className="chip" onClick={() => { setInput(x); void check(x); }}>{x}</button>)}
            </div>
          )}
          {error && <div className="notice bad-text">{error}</div>}
          {r && leaf && (
            <>
              <div className={`tls-verdict ${r.trusted ? "ok" : "bad"}`}>
                {r.trusted ? <CheckCircle2 size={18} className="ok-text" /> : <ShieldAlert size={18} className="bad-text" />}
                <div>
                  <b>{r.trusted ? "Trusted: browsers accept this certificate" : "Not trusted"}</b>
                  {r.problem && <div className="muted">{r.problem}</div>}
                  <div className="faint mono" style={{ fontSize: 12 }}>{r.host}:{r.port}{r.address ? ` (${r.address})` : ""} · {r.protocol ?? "?"} · {r.cipher ?? "?"}</div>
                </div>
                <span className={`badge ${leaf.daysLeft < 0 ? "bad" : leaf.daysLeft <= 14 ? "warn" : "ok"}`}>{leaf.daysLeft < 0 ? "expired" : `${leaf.daysLeft} days left`}</span>
              </div>
              {r.chain.map((c, i) => (
                <div key={c.serial + i} className="tls-cert">
                  <div className="tls-cert-head">
                    <Lock size={13} className="faint" />
                    <b>{i === 0 ? "Server certificate" : `Issuer ${i}`}</b>
                    <span className="mono">{c.commonName ?? c.subject}</span>
                    {c.selfSigned && <span className="badge warn">self-signed</span>}
                  </div>
                  <dl className="kv">
                    {c.names.length > 0 && <><dt>Valid for</dt><dd className="mono">{c.names.join("\n")}</dd></>}
                    <dt>Issued by</dt><dd>{c.issuer}</dd>
                    <dt>Valid</dt><dd>{day(c.notBefore)} – {day(c.notAfter)} {c.daysLeft < 0 ? <XCircle size={12} className="bad-text" /> : null}</dd>
                    <dt>Key</dt><dd>{c.key}</dd>
                    <dt>Serial</dt><dd className="mono">{c.serial}</dd>
                  </dl>
                </div>
              ))}
              <p className="footnote">Checked from this computer, the way a browser would (public certificate authorities and the host name). Only the TLS handshake is made; nothing is requested. A missing intermediate certificate shows up here as "isn't signed by a public certificate authority" even though some browsers fill the gap themselves.</p>
            </>
          )}
          {!r && !error && <p className="muted">Look at any HTTPS, mail (993, 465, 995) or other TLS server's certificate: who issued it, which names it covers, when it expires, and whether browsers trust it. Use it after changing a certificate, or to check a server you don't manage.</p>}
        </div>
      </div>
    </div>
  );
}
