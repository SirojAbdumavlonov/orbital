import { AlertTriangle, CheckCircle2, RefreshCw, XCircle } from "lucide-react";
import { useEffect, useState } from "react";
import { api } from "../lib/api";
import { checkup, type Finding } from "../lib/checkup";
import { SessionState, useStore } from "../store";

/** One-click health check: what needs attention on this server, worst first, each with the tab that fixes it. */
export function CheckupView({ s }: { s: SessionState }) {
  const sid = s.info.id;
  const [found, setFound] = useState<Finding[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [at, setAt] = useState<number | null>(null);
  const has = (id: string) => !!s.tools?.some((t) => t.id === id);

  async function run() {
    setBusy(true);
    const ok = <T,>(p: Promise<T>) => p.catch(() => null);
    const [info, sshd, ufw, certs, containers] = await Promise.all([
      ok(api.systemInfo(sid)),
      ok(api.sshdReport(sid)),
      s.tools === null ? Promise.resolve(null) : has("ufw") ? ok(api.ufwStatus(sid)) : Promise.resolve("absent" as const),
      has("certbot") ? ok(api.certbotStatus(sid)) : Promise.resolve(null),
      s.info.dockerVersion ? ok(api.listContainers(sid)) : Promise.resolve(null),
    ]);
    const all = useStore.getState().sessions[sid]?.metrics ?? [];
    const metrics = all[all.length - 1] ?? null;
    setFound(checkup({ info, metrics, sshd, ufw, certs, containers }));
    setAt(Date.now());
    setBusy(false);
  }
  useEffect(() => { void run(); }, [sid, s.tools === null]);

  const go = (tab: string) => {
    const st = useStore.getState();
    if (tab.startsWith("tool:")) st.openTool(sid, tab.slice(5));
    else st.setTab(sid, tab as never);
  };
  const bad = found?.filter((f) => f.level === "bad").length ?? 0;
  const warn = found?.filter((f) => f.level === "warn").length ?? 0;
  return (
    <div className="view">
      <div className="toolbar">
        {found && (bad + warn === 0
          ? <span className="badge ok">All good</span>
          : <span className={`badge ${bad ? "bad" : "amber"}`}>{bad ? `${bad} to fix` : ""}{bad && warn ? " · " : ""}{warn ? `${warn} to look at` : ""}</span>)}
        {at && <span className="faint">checked {new Date(at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}</span>}
        <span className="spacer" />
        <button className="btn" disabled={busy} onClick={run}><RefreshCw size={13} className={busy ? "spin" : ""} /> Check again</button>
      </div>
      <div className="scroll">
        {!found && <div className="empty"><div><RefreshCw size={22} className="spin faint" /><p>Looking at disks, memory, services, containers, updates, the firewall, SSH and certificates…</p></div></div>}
        <ul className="checkup">
          {found?.map((f, i) => {
            const Icon = f.level === "bad" ? XCircle : f.level === "warn" ? AlertTriangle : CheckCircle2;
            return (
              <li key={i} className={`checkup-item ${f.level}`}>
                <Icon size={16} />
                <div className="checkup-text">
                  <b>{f.title}</b>
                  <span className="faint">{f.detail}</span>
                </div>
                {f.go && f.level !== "ok" && <button className="btn" onClick={() => go(f.go!.tab)}>{f.go.label}</button>}
              </li>
            );
          })}
        </ul>
      </div>
    </div>
  );
}
