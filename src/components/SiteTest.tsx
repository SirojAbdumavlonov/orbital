import { CheckCircle2, CircleDashed, RefreshCw, XCircle } from "lucide-react";
import { useEffect, useState } from "react";
import { api, asApiError, SiteProbe, SiteTest } from "../lib/api";

const good = (p: SiteProbe) => p.status != null && p.status < 500;

function Row({ label, p, hint }: { label: string; p: SiteProbe; hint: string }) {
  const ok = good(p);
  return (
    <div className="site-test-row">
      {ok ? <CheckCircle2 size={14} className="ok-text" /> : <XCircle size={14} className="bad-text" />}
      <div>
        <b>{label}</b> <span className="mono faint">{p.url}</span>
        <div className={ok ? "faint" : "bad-text"}>{p.status != null ? `HTTP ${p.status}${p.status >= 300 && p.status < 400 ? " (a redirect, e.g. to https)" : ""}${p.status >= 500 ? ": the site answers with an error" : ""}` : p.error}</div>
        {!ok && <div className="faint">{hint}</div>}
      </div>
    </div>
  );
}

/**
 * Is the website really working? nginx's own check, a request on the server
 * (is nginx serving this name?) and one from this computer (are DNS and the
 * firewalls right?).
 */
export function SiteTestPanel({ sid, domain, https }: { sid: string; domain: string; https: boolean }) {
  const [t, setT] = useState<SiteTest | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const run = () => {
    setBusy(true);
    setErr(null);
    api.websiteTest(sid, domain, https).then(setT, (e) => setErr(asApiError(e).message)).finally(() => setBusy(false));
  };
  useEffect(run, [sid, domain, https]);
  return (
    <div className="site-test">
      <div className="site-test-head">
        <b>Test {https ? "https" : "http"}://{domain}</b>
        <span className="spacer" />
        <button className="icon-btn" aria-label="Test again" title="Test again" onClick={run}><RefreshCw size={13} className={busy ? "spin" : ""} /></button>
      </div>
      {busy && !t && <div className="faint"><CircleDashed size={13} className="spin" /> Testing…</div>}
      {err && <div className="bad-text">{err}</div>}
      {t && (
        <>
          <div className="site-test-row">
            {t.configOk ? <CheckCircle2 size={14} className="ok-text" /> : <XCircle size={14} className="bad-text" />}
            <div>
              <b>nginx configuration</b> <span className="faint">{t.configOk ? "nginx -t passed" : "nginx -t found errors"}</span>
              {!t.configOk && <pre className="output">{t.configOutput}</pre>}
            </div>
          </div>
          <Row label="On the server" p={t.server} hint="nginx isn't serving this name: check the site's server_name and that nginx was reloaded." />
          <Row label="From this computer" p={t.internet} hint={good(t.server) ? "The server serves it, so the way in is blocked: the domain's DNS may not point here yet, or a firewall (ufw, or the cloud provider's, e.g. Oracle's security list or Hetzner's firewall) closes the port." : "Fix the server side first."} />
        </>
      )}
    </div>
  );
}
