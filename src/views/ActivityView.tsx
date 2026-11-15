import { save } from "@tauri-apps/plugin-dialog";
import { Download, RefreshCw, Search, ShieldAlert, ShieldCheck } from "lucide-react";
import { Fragment, useEffect, useState } from "react";
import { api, asApiError, AuditCategory, AuditEvent } from "../lib/api";
import { useStore } from "../store";

const CATS: (AuditCategory | "all")[] = ["all", "config", "file", "service", "docker", "command", "terminal", "connection", "security"];
const CAT_LABEL: Record<string, string> = {
  all: "All", config: "Config", file: "Files", service: "Services", docker: "Docker", command: "Commands",
  terminal: "Terminal", connection: "Sessions", security: "Security",
};

export function ActivityView({ profileId }: { profileId?: string }) {
  const toast = useStore((x) => x.toast);
  const [events, setEvents] = useState<AuditEvent[]>([]);
  const [cat, setCat] = useState<AuditCategory | "all">("all");
  const [q, setQ] = useState("");
  const [open, setOpen] = useState<number | null>(null);
  const [verify, setVerify] = useState<{ ok: boolean; checked: number; firstBrokenSeq: number | null } | null>(null);

  const load = () =>
    api.auditQuery({ profileId, category: cat === "all" ? undefined : cat, search: q || undefined, limit: 500 }).then(setEvents).catch(() => {});
  useEffect(() => { load(); }, [cat, q, profileId]);
  useEffect(() => { api.auditVerify().then(setVerify).catch(() => {}); }, []);

  async function exportLog(format: "csv" | "json") {
    const path = await save({ defaultPath: `orbital-activity-${new Date().toISOString().slice(0, 10)}.${format}`, title: "Export activity log" });
    if (!path) return;
    try {
      await api.auditExport(format, path);
      toast({ kind: "success", title: "Activity log exported", body: path });
    } catch (e) {
      toast({ kind: "error", title: "Export failed", body: asApiError(e).message });
    }
  }

  return (
    <div className="view">
      <div className="toolbar">
        <select className="input" style={{ width: 140, height: 28 }} value={cat} onChange={(e) => setCat(e.target.value as any)} aria-label="Category">
          {CATS.map((c) => <option key={c} value={c}>{CAT_LABEL[c]}</option>)}
        </select>
        <div className="search"><Search size={14} /><input className="input" placeholder="Search actions, files, commands" value={q} onChange={(e) => setQ(e.target.value)} /></div>
        <span className="spacer" />
        {verify && (verify.ok
          ? <span className="badge ok" title="Every entry's SHA-256 chain was re-computed and matches"><ShieldCheck size={12} /> {verify.checked} entries verified</span>
          : <span className="badge bad"><ShieldAlert size={12} /> Log altered at entry #{verify.firstBrokenSeq}</span>)}
        <button className="btn" onClick={() => api.auditVerify().then(setVerify)}>Verify</button>
        <button className="btn" onClick={() => exportLog("csv")}><Download size={13} /> CSV</button>
        <button className="btn" onClick={() => exportLog("json")}><Download size={13} /> JSON</button>
        <button className="icon-btn" aria-label="Refresh" onClick={load}><RefreshCw size={14} /></button>
      </div>
      <div className="scroll">
        <table className="table">
          <thead><tr><th>When</th>{!profileId && <th>Server</th>}<th>Action</th><th>Target</th><th>Result</th><th className="num">Time</th></tr></thead>
          <tbody>
            {events.map((e) => (
              <Fragment key={e.seq}>
                <tr className={`click ${open === e.seq ? "sel" : ""}`} onClick={() => setOpen(open === e.seq ? null : e.seq)}>
                  <td className="faint" style={{ whiteSpace: "nowrap" }}>{new Date(e.ts).toLocaleString()}</td>
                  {!profileId && <td className="mono muted" style={{ fontSize: 12 }}>{e.server ?? "—"}</td>}
                  <td className="mono">{e.action}</td>
                  <td className="mono muted" style={{ maxWidth: 380, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", fontSize: 12 }}>{e.target ?? ""}</td>
                  <td>
                    <span className={`badge ${e.outcome === "success" ? "ok" : e.outcome === "denied" ? "amber" : "bad"}`}>
                      {e.outcome === "success" ? "Done" : e.outcome === "denied" ? "Blocked" : "Failed"}{e.exitCode != null && e.exitCode !== 0 ? ` (${e.exitCode})` : ""}
                    </span>
                  </td>
                  <td className="num faint">{e.durationMs != null ? `${e.durationMs} ms` : ""}</td>
                </tr>
                {open === e.seq && (
                  <tr>
                    <td colSpan={profileId ? 5 : 6} style={{ background: "var(--slate-2)" }}>
                      <dl className="kv" style={{ padding: "4px 0" }}>
                        <dt>Actor</dt><dd>{e.actor} (this Mac) as {e.server?.split("@")[0] ?? "—"}</dd>
                        {e.command && <><dt>Command sent</dt><dd className="mono">{e.command}</dd></>}
                        {e.details && <><dt>Details</dt><dd className="mono">{JSON.stringify(e.details, null, 1)}</dd></>}
                        <dt>Entry hash</dt><dd className="mono faint">{e.hash}</dd>
                        <dt>Previous hash</dt><dd className="mono faint">{e.prevHash}</dd>
                      </dl>
                    </td>
                  </tr>
                )}
              </Fragment>
            ))}
          </tbody>
        </table>
        {events.length === 0 && <div className="empty"><p>Nothing recorded{q || cat !== "all" ? " for this filter" : " yet"}. Every change you make on a server will appear here.</p></div>}
      </div>
    </div>
  );
}
