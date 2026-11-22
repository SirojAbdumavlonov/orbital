import { CheckCircle2, ChevronDown, ChevronRight, Loader2, Play, XCircle } from "lucide-react";
import { useEffect, useState } from "react";
import { api, asApiError, NetCheck } from "../lib/api";
import { useKept } from "../lib/kept";
import { useNetRuns } from "../lib/netHistory";
import { SessionState } from "../store";

const CHECKS: { id: NetCheck; label: string; hint: string; placeholder: string }[] = [
  { id: "port", label: "Port", hint: "Can the server open a connection to this port? (database, cache, another service)", placeholder: "db.internal or 10.0.0.5" },
  { id: "http", label: "HTTP(S)", hint: "Does a web address answer from here, with which status and how fast?", placeholder: "https://api.example.com/health" },
  { id: "dns", label: "DNS", hint: "What does this name resolve to on the server?", placeholder: "example.com" },
  { id: "ping", label: "Ping", hint: "Does the host answer ping, and how far away is it?", placeholder: "1.1.1.1" },
  { id: "trace", label: "Traceroute", hint: "Which way do packets go? (up to 20 hops, about 40 s at most)", placeholder: "example.com" },
];

const KEY = (sid: string) => `orbital.netcheck.${sid}`;
function recent(profileId: string): { check: NetCheck; target: string; port: number | null }[] {
  try {
    return JSON.parse(localStorage.getItem(KEY(profileId)) ?? "[]");
  } catch {
    return [];
  }
}

/** Time today, or day and time for older results. */
function when(ms: number): string {
  const d = new Date(ms);
  const t = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  return d.toDateString() === new Date().toDateString() ? t : `${d.toLocaleDateString([], { month: "short", day: "numeric" })} ${t}`;
}

/** Network check: is a host, port or web address reachable from this server? */
export function NetworkView({ s }: { s: SessionState }) {
  const sid = s.info.id;
  const pid = s.info.profileId;
  // What you were typing and which result was open come back with the tab.
  const [check, setCheck] = useKept<NetCheck>(`${sid}:net:check`, "port");
  const [target, setTarget] = useKept(`${sid}:net:target`, "");
  const [port, setPort] = useKept(`${sid}:net:port`, "");
  const [open, setOpen] = useKept<number | null>(`${sid}:net:open`, null);
  // Results live outside the view (lib/netHistory), so a running check isn't lost on a tab switch.
  const runs = useNetRuns((x) => x.byProfile[pid]) ?? [];
  useEffect(() => { useNetRuns.getState().list(pid); }, [pid]);
  const [saved, setSaved] = useState(() => recent(pid));
  const spec = CHECKS.find((c) => c.id === check)!;

  async function run(c = check, t = target.trim(), p = port ? Number(port) : null) {
    if (!t) return;
    const n = Date.now();
    const store = useNetRuns.getState();
    store.add(pid, { n, check: c, target: t, port: p, at: n, busy: true });
    const keep = [{ check: c, target: t, port: p }, ...saved.filter((r) => !(r.check === c && r.target === t && r.port === p))].slice(0, 8);
    setSaved(keep);
    try { localStorage.setItem(KEY(pid), JSON.stringify(keep)); } catch { /* not remembered */ }
    try {
      const result = await api.netCheck(sid, c, t, c === "port" ? p : null);
      store.update(pid, n, { result, busy: false });
    } catch (e) {
      store.update(pid, n, { error: asApiError(e).message, busy: false });
    }
  }

  const label = (r: { check: NetCheck; target: string; port: number | null }) => `${CHECKS.find((c) => c.id === r.check)?.label} ${r.target}${r.check === "port" && r.port ? `:${r.port}` : ""}`;

  return (
    <div className="view">
      <form className="net-form" onSubmit={(e) => { e.preventDefault(); void run(); }}>
        <div className="segmented">
          {CHECKS.map((c) => <button type="button" key={c.id} className={check === c.id ? "on" : ""} onClick={() => setCheck(c.id)}>{c.label}</button>)}
        </div>
        <div className="net-inputs">
          <input className="input mono" value={target} onChange={(e) => setTarget(e.target.value)} placeholder={spec.placeholder} autoFocus spellCheck={false} />
          {check === "port" && <input className="input mono net-port" value={port} onChange={(e) => setPort(e.target.value.replace(/\D/g, "").slice(0, 5))} placeholder="5432" inputMode="numeric" />}
          <button className="btn primary" disabled={!target.trim() || (check === "port" && !port)}><Play size={13} /> Check</button>
        </div>
        <p className="faint net-hint">{spec.hint} Runs on {s.info.label}, not on this computer.</p>
        {saved.length > 0 && (
          <div className="chips">
            {saved.map((r) => (
              <button type="button" key={label(r)} className="chip mono" onClick={() => { setCheck(r.check); setTarget(r.target); setPort(r.port ? String(r.port) : ""); void run(r.check, r.target, r.port); }}>{label(r)}</button>
            ))}
          </div>
        )}
      </form>
      <div className="scroll">
        {runs.length > 0 && (
          <div className="net-history-head">
            <span className="faint">Results on {s.profile.name} (kept on this computer, newest first)</span>
            <span className="spacer" />
            <button className="btn ghost" onClick={() => { useNetRuns.getState().clear(pid); setOpen(null); }}>Clear</button>
          </div>
        )}
        {runs.length === 0 && <div className="empty"><p>Check whether the server can reach a database, an API or another server, and see where it fails: name lookup, the network or the service itself.</p></div>}
        {runs.map((r) => {
          const ok = r.result?.ok;
          const busy = !!r.busy && !r.result && !r.error;
          return (
            <div key={r.n} className="net-run">
              <button className="net-row" onClick={() => setOpen(open === r.n ? null : r.n)} disabled={busy}>
                {busy ? <Loader2 size={15} className="spin faint" /> : ok ? <CheckCircle2 size={15} className="ok-ic" /> : <XCircle size={15} className="bad-ic" />}
                <span className="mono">{label(r)}</span>
                <span className={busy ? "faint" : ok ? "" : "bad-text"}>{busy ? "checking…" : r.error ?? r.result?.summary}</span>
                <span className="spacer" />
                <span className="faint">{when(r.at)}</span>
                {!busy && r.result && (open === r.n ? <ChevronDown size={14} /> : <ChevronRight size={14} />)}
              </button>
              {open === r.n && r.result && <pre className="output net-out">{`$ ${r.result.command}\n${r.result.output}`}</pre>}
            </div>
          );
        })}
      </div>
    </div>
  );
}
