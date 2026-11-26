import { AlertTriangle, CheckCircle2, Info, RefreshCw, ShieldCheck, XCircle } from "lucide-react";
import { useState } from "react";
import { api, AuditReport } from "../lib/api";
import { runJob, useRunningJob } from "../lib/jobs";
import { useKept } from "../lib/kept";
import { SessionState } from "../store";

const ICON = { bad: XCircle, warn: AlertTriangle, info: Info, ok: CheckCircle2 };
const COLOR = { bad: "var(--bad)", warn: "var(--warn)", info: "var(--ink-3)", ok: "var(--ok)" };

/**
 * Security audit: one read-only scan for the problems attackers look for
 * first (extra root accounts, empty passwords, setuid programs,
 * world-writable files, exposed databases, passwordless sudo, key files).
 */
export function SecurityAuditView({ s }: { s: SessionState }) {
  const sid = s.info.id;
  const key = `${sid}:secaudit`;
  const [r, setR] = useKept<AuditReport | null>(`${sid}:secaudit:report`, null);
  const [open, setOpen] = useState<string | null>(null);
  const running = useRunningJob(key);

  async function scan() {
    try {
      setR(await runJob((p) => api.securityAudit(sid, p), {
        key, title: "Security audit", sid, quietHere: true,
        done: (rep) => {
          const bad = rep.findings.filter((f) => f.level === "bad").length;
          return bad ? `${bad} serious problems found` : "No serious problems found";
        },
      }));
    } catch {
      /* the jobs layer reports it */
    }
  }

  const counts = (r?.findings ?? []).reduce((m, f) => ({ ...m, [f.level]: (m[f.level] ?? 0) + 1 }), {} as Record<string, number>);
  return (
    <div className="view">
      <div className="toolbar view-toolbar">
        {r ? <span className="faint">{counts.bad ?? 0} serious · {counts.warn ?? 0} to look at · {counts.ok ?? 0} fine</span> : <span className="faint" style={{ fontSize: 12.5 }}>A read-only scan for what attackers look for first.</span>}
        <span className="spacer" />
        <button className="btn primary" disabled={!!running} onClick={() => void scan()}><RefreshCw size={13} className={running ? "spin" : ""} /> {running ? "Scanning…" : r ? "Scan again" : "Scan this server"}</button>
      </div>
      <div className="scroll">
        {!r && !running && (
          <div className="empty"><div>
            <ShieldCheck size={28} className="faint" />
            <h3>Security audit</h3>
            <p>Looks for extra root accounts, empty passwords, unusual setuid programs, world-writable files, databases listening on every address, passwordless sudo and SSH key files others can change. Nothing on the server is changed; it takes a few seconds.</p>
            <button className="btn primary" onClick={() => void scan()}>Scan this server</button>
          </div></div>
        )}
        {r?.partial && <div className="notice">Part of the scan needs root (the password file and other users' files). Turn on sudo for this server and scan again for the full picture.</div>}
        {r && (
          <div className="checks">
            {r.findings.map((f) => {
              const Icon = ICON[f.level];
              const expanded = open === f.id || f.items.length <= 3;
              return (
                <div key={f.id} className="check-card">
                  <Icon size={16} color={COLOR[f.level]} style={{ flex: "none", marginTop: 2 }} />
                  <div style={{ minWidth: 0, flex: 1 }}>
                    <div className="check-title"><b>{f.title}</b>{f.items.length > 0 && <span className="faint">{f.items.length}</span>}</div>
                    <div className="muted">{f.detail}</div>
                    {f.items.length > 0 && (
                      <ul className="audit-items mono">
                        {(expanded ? f.items : f.items.slice(0, 3)).map((x) => <li key={x}>{x}</li>)}
                        {!expanded && <li><button className="link-btn" onClick={() => setOpen(f.id)}>Show all {f.items.length}</button></li>}
                      </ul>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        )}
        {r && <p className="footnote">Read-only: nothing was changed. Fix accounts in the Users tab, files in Files (chmod), sudo rules in the editor, and exposed ports in Open ports or the Firewall tab.</p>}
      </div>
    </div>
  );
}
