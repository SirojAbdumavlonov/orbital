import { CalendarClock, FileText, Folder, Pause, Pencil, Play, Plus, RefreshCw, Search, Trash2, Zap } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { confirm, DrawerClose, Modal, StateDot } from "../components/ui";
import { api, asApiError, CronJob, CronPreview, CronReport, ExecOutput } from "../lib/api";
import { useRemote } from "../lib/remote";
import { SessionState, useStore } from "../store";
import { WithServicePane } from "./DatabaseView";

/** "in 3 h", "in 12 min", "tomorrow 03:00" — relative to the server's clock. */
function when(iso: string | null | undefined, nowIso: string | null): string {
  if (!iso) return "—";
  const t = new Date(iso).getTime();
  const now = nowIso ? new Date(nowIso).getTime() : Date.now();
  const diff = Math.round((t - now) / 60000);
  const abs = Math.abs(diff);
  const span = abs < 60 ? `${abs} min` : abs < 48 * 60 ? `${Math.round(abs / 60)} h` : `${Math.round(abs / 1440)} days`;
  return diff >= 0 ? `in ${span}` : `${span} ago`;
}

/** The server's wall-clock time of an RFC 3339 stamp, e.g. "Mon 16 Nov 03:00". */
function clock(iso: string | null | undefined): string {
  if (!iso) return "";
  const m = iso.match(/^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})/);
  if (!m) return iso;
  const d = new Date(Date.UTC(+m[1], +m[2] - 1, +m[3]));
  return `${d.toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short", timeZone: "UTC" })} ${m[4]}:${m[5]}`;
}

/** systemd's "Mon 2026-11-16 00:00:00 UTC" → "Mon 16 Nov 00:00". */
function stamp(v: string | null): string {
  if (!v) return "—";
  const m = v.match(/^(\w{3}) (\d{4})-(\d{2})-(\d{2}) (\d{2}:\d{2})/);
  if (!m) return v;
  const month = new Date(Date.UTC(+m[2], +m[3] - 1, 1)).toLocaleDateString(undefined, { month: "short", timeZone: "UTC" });
  return `${m[1]} ${+m[4]} ${month} ${m[5]}`;
}

const PRESETS: [string, string][] = [
  ["Every 5 minutes", "*/5 * * * *"],
  ["Every hour", "0 * * * *"],
  ["Every day at 03:00", "0 3 * * *"],
  ["Weekdays at 09:00", "0 9 * * 1-5"],
  ["Every Sunday at 02:00", "0 2 * * 0"],
  ["1st of the month", "0 4 1 * *"],
  ["When the server starts", "@reboot"],
];

function JobDialog({ s, report, job, onClose, onSaved }: { s: SessionState; report: CronReport; job: CronJob | null; onClose: () => void; onSaved: (r: CronReport) => void }) {
  const toast = useStore((x) => x.toast);
  const me = s.profile.username;
  const users = [...new Set([job?.user ?? me, me, ...(s.info.elevation || me === "root" ? ["root"] : []), ...report.jobs.filter((j) => j.source === "user").map((j) => j.user)])];
  const [user, setUser] = useState(job?.user ?? me);
  const [schedule, setSchedule] = useState(job?.schedule ?? "0 3 * * *");
  const [command, setCommand] = useState(job?.command ?? "");
  const [note, setNote] = useState(job?.note ?? "");
  const [preview, setPreview] = useState<CronPreview | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!report.serverNow) return;
    const t = setTimeout(() => api.cronPreview(schedule, report.serverNow!).then(setPreview).catch(() => setPreview(null)), 150);
    return () => clearTimeout(t);
  }, [schedule]);

  async function save() {
    setBusy(true);
    try {
      const n = note.trim() || null;
      const r = await api.cronChange(s.info.id, user, job ? { type: "update", raw: job.raw, schedule, command, note: n } : { type: "add", schedule, command, note: n });
      toast({ kind: "success", title: job ? "Job changed" : "Job added", body: preview?.description });
      onSaved(r);
      onClose();
    } catch (e) {
      toast({ kind: "error", title: job ? "Couldn't change the job" : "Couldn't add the job", body: asApiError(e).message });
    } finally {
      setBusy(false);
    }
  }

  const ok = !!command.trim() && !preview?.error;
  return (
    <Modal title={job ? "Edit job" : "Add a scheduled job"} wide subtitle={`Runs on ${s.profile.name}, in the server's time${report.timezone ? ` (${report.timezone})` : ""}.`} onClose={busy ? undefined : onClose}
      footer={<><button className="btn" disabled={busy} onClick={onClose}>Cancel</button><button className="btn primary" disabled={!ok || busy} onClick={save}>{busy ? "Saving…" : job ? "Save job" : "Add job"}</button></>}>
      <div className="form">
        <div className="field"><span>When</span>
          <div className="chips">{PRESETS.map(([label, v]) => <button key={v} className={`chip ${schedule === v ? "on" : ""}`} onClick={() => setSchedule(v)}>{label}</button>)}</div>
          <input className="input mono" value={schedule} onChange={(e) => setSchedule(e.target.value)} aria-label="Schedule" />
          <small className="hint">
            {preview?.error ? <span className="bad-text">{preview.error}</span>
              : preview ? <><b>{preview.description}</b>{preview.nextRuns.length > 0 && <> · next: {preview.nextRuns.map(clock).join(", ")}</>}</>
              : "minute hour day-of-month month day-of-week, e.g. 30 2 * * 1-5"}
          </small>
        </div>
        <label className="field"><span>Command</span>
          <input className="input mono" value={command} onChange={(e) => setCommand(e.target.value)} placeholder="/usr/local/bin/backup.sh >> /var/log/backup.log 2>&1" />
          <small className="hint">Runs with /bin/sh, with a minimal PATH: use full paths. Without a redirect, output is mailed to the user (or lost). In cron, % means a new line: write \% for a literal one.</small>
        </label>
        <div className="row2" style={{ gridTemplateColumns: "1fr 2fr" }}>
          <label className="field"><span>Runs as</span>
            <select className="input mono" value={user} disabled={!!job} onChange={(e) => setUser(e.target.value)}>
              {users.map((u) => <option key={u}>{u}</option>)}
            </select>
          </label>
          <label className="field"><span>Note (optional)</span>
            <input className="input" value={note} onChange={(e) => setNote(e.target.value)} placeholder="What this job is for" />
          </label>
        </div>
      </div>
    </Modal>
  );
}

/** Every scheduled job on the server: when it runs, what it runs, and whether it ran. */
export function CronView({ s }: { s: SessionState }) {
  const { toast, openFile, showFolder } = useStore();
  const sid = s.info.id;
  const { data: r, setData, error, loading, reload } = useRemote(() => api.cronReport(sid), [sid]);
  const [filter, setFilter] = useState<"all" | "user" | "system">("all");
  const [q, setQ] = useState("");
  const [sel, setSel] = useState<string | null>(null);
  const [editing, setEditing] = useState<CronJob | "new" | null>(null);
  const [out, setOut] = useState<{ title: string; o: ExecOutput } | null>(null);
  const [busy, setBusy] = useState(false);

  const key = (j: CronJob) => `${j.source}:${j.file ?? j.user}:${j.line}`;
  const jobs = useMemo(() => (r?.jobs ?? []).filter((j) => (filter === "all" || j.source === filter) && (!q || `${j.command} ${j.user} ${j.note ?? ""} ${j.description}`.toLowerCase().includes(q.toLowerCase()))), [r, filter, q]);
  const cur = r?.jobs.find((j) => key(j) === sel) ?? null;

  async function change(job: CronJob, c: Parameters<typeof api.cronChange>[2], done: string) {
    setBusy(true);
    try {
      setData(await api.cronChange(sid, job.user, c));
      toast({ kind: "success", title: done });
      return true;
    } catch (e) {
      toast({ kind: "error", title: "Couldn't change the job", body: asApiError(e).message });
      reload();
      return false;
    } finally {
      setBusy(false);
    }
  }

  async function runNow(job: CronJob) {
    const ok = await confirm({ title: "Run this job now?", body: <>It runs once as <b>{job.user}</b>, the way cron would, and Orbital shows what it prints (up to 10 minutes).<div className="mono" style={{ marginTop: 8 }}>{job.command}</div></>, confirmLabel: "Run now" });
    if (!ok) return;
    setBusy(true);
    try {
      const o = await api.cronRun(sid, job.user, job.command);
      setOut({ title: o.exitCode === 0 ? "The job finished" : `The job failed (exit ${o.exitCode})`, o });
    } catch (e) {
      toast({ kind: "error", title: "Couldn't run the job", body: asApiError(e).message });
    } finally {
      setBusy(false);
    }
  }

  async function startTimer(unit: string, activates: string) {
    try {
      const rep = await api.serviceAction(sid, activates, "start");
      toast(rep.output.exitCode === 0 ? { kind: "success", title: `${activates} started`, body: `What ${unit} runs, started once now.` } : { kind: "error", title: `${activates} failed`, body: rep.output.stderr });
    } catch (e) {
      toast({ kind: "error", title: `Couldn't start ${activates}`, body: asApiError(e).message });
    }
  }

  const periodic = useMemo(() => {
    const g: Record<string, string[]> = {};
    for (const p of r?.periodic ?? []) (g[p.period] ??= []).push(p.path);
    return Object.entries(g);
  }, [r]);

  return (
    <WithServicePane s={s} toolId="cron" label="Jobs">
      <div className="toolbar">
        <div className="segmented">
          {([["all", "All"], ["user", "Users' crontabs"], ["system", "System files"]] as const).map(([id, label]) => <button key={id} className={filter === id ? "on" : ""} onClick={() => setFilter(id)}>{label}</button>)}
        </div>
        <div className="search"><Search size={14} /><input className="input" placeholder="Find a command or user" value={q} onChange={(e) => setQ(e.target.value)} /></div>
        <span className="spacer" />
        {r?.serverNow && <span className="faint" title="Cron uses the server's clock">Server time {clock(r.serverNow).split(" ").pop()}{r.timezone ? ` · ${r.timezone}` : ""}</span>}
        {r && <button className="btn primary" onClick={() => setEditing("new")}><Plus size={13} /> Add job</button>}
        <button className="icon-btn" aria-label="Refresh" onClick={reload}><RefreshCw size={14} className={loading ? "spin" : ""} /></button>
      </div>
      {error && <div className="empty"><div><h3>Couldn't read the scheduled jobs</h3><p>{error.message}</p></div></div>}
      {r && !r.complete && <div className="notice">Only your own crontab is shown: other users' jobs need root. Turn on sudo for this server, then reconnect.</div>}
      {r && (
        <div className="split">
          <div className="scroll">
            <table className="table">
              <thead><tr><th>When</th><th>Command</th><th>User</th><th className="nowrap">Next run</th><th className="nowrap">Last run</th></tr></thead>
              <tbody>
                {jobs.map((j) => (
                  <tr key={key(j)} className={`click ${sel === key(j) ? "sel" : ""} ${j.paused ? "dim" : ""}`} onClick={() => setSel(key(j))}>
                    <td>
                      <div style={{ minWidth: 170 }}>{j.paused ? <span className="badge">Paused</span> : <b>{j.description}</b>}</div>
                      <div className="mono faint" style={{ fontSize: 11.5, whiteSpace: "nowrap" }}>{j.schedule}</div>
                    </td>
                    <td className="mono">
                      <div className="cell-clip" style={{ maxWidth: 220 }} title={j.command}>{j.command}</div>
                      {j.note && <div className="faint" style={{ fontSize: 12, fontFamily: "var(--sans)" }}>{j.note}</div>}
                    </td>
                    <td className="mono muted">{j.user}{j.source === "system" && <div className="faint" style={{ fontSize: 11 }}>{j.file?.split("/").pop()}</div>}</td>
                    <td className="nowrap">{j.nextRuns[0] ? <span title={clock(j.nextRuns[0])}>{when(j.nextRuns[0], r.serverNow)}</span> : <span className="faint">{j.paused ? "—" : j.schedule === "@reboot" ? "at startup" : "—"}</span>}</td>
                    <td className="nowrap faint">{j.lastRun ? when(j.lastRun.replace(/([+-]\d{2})(\d{2})$/, "$1:$2"), r.serverNow) : "—"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            {jobs.length === 0 && <div className="empty" style={{ flex: "none" }}><div><CalendarClock size={26} className="faint" /><p>{q || filter !== "all" ? "No jobs match." : "No cron jobs yet."}</p></div></div>}

            {r.timers.length > 0 && <>
              <div className="section-title">Systemd timers <span className="faint">(the newer way services schedule work, e.g. certificate renewal and updates)</span></div>
              <table className="table">
                <thead><tr><th>Timer</th><th>Schedule</th><th>Next</th><th>Last</th><th /></tr></thead>
                <tbody>
                  {r.timers.map((t) => (
                    <tr key={t.unit}>
                      <td title={t.description ?? ""}><span className="mono">{t.unit.replace(/\.timer$/, "")}</span></td>
                      <td className="mono faint nowrap">{t.calendar ?? "—"}</td>
                      <td className="faint nowrap" title={t.next ?? ""}>{stamp(t.next)}</td>
                      <td className="faint nowrap" title={t.last ?? ""}>{stamp(t.last)}</td>
                      <td style={{ textAlign: "right" }}><button className="icon-btn" aria-label={`Run ${t.activates} now`} title={`Start ${t.activates} once now`} onClick={() => startTimer(t.unit, t.activates)}><Zap size={14} /></button></td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </>}

            {periodic.length > 0 && <>
              <div className="section-title">Hourly, daily, weekly and monthly scripts</div>
              <table className="table"><tbody>
                {periodic.map(([period, paths]) => (
                  <tr key={period}>
                    <td style={{ width: 150 }}><a className="link" onClick={() => showFolder(sid, `/etc/cron.${period}`)}><Folder size={13} style={{ verticalAlign: -2 }} /> /etc/cron.{period}</a></td>
                    <td className="mono">{paths.map((p) => <a key={p} className="link" style={{ marginRight: 12 }} onClick={() => openFile(sid, p)}>{p.split("/").pop()}</a>)}</td>
                  </tr>
                ))}
              </tbody></table>
            </>}
            <p className="footnote">Jobs in users' crontabs are changed through the crontab command, which checks the syntax first. Jobs in /etc/crontab and /etc/cron.d are edited in their file. Paused jobs stay in the crontab, commented out.</p>
          </div>
          <aside className="drawer">
            {!cur ? <div className="empty"><p>Select a job to run it now, change, pause or delete it.</p></div> : (
              <>
                <div className="drawer-head">
                  <DrawerClose onClose={() => setSel(null)} />
                  <h3>{cur.paused ? "Paused" : cur.description}</h3>
                  <div className="muted mono" style={{ fontSize: 12 }}>{cur.schedule}</div>
                </div>
                <div className="actions">
                  <button className="btn" disabled={busy} onClick={() => runNow(cur)}><Play size={13} /> Run now</button>
                  {cur.source === "user" ? <>
                    <button className="btn" disabled={busy} onClick={() => setEditing(cur)}><Pencil size={13} /> Edit</button>
                    {cur.paused
                      ? <button className="btn" disabled={busy} onClick={() => change(cur, { type: "resume", raw: cur.raw }, "Job resumed")}><Play size={13} /> Resume</button>
                      : <button className="btn" disabled={busy} onClick={() => change(cur, { type: "pause", raw: cur.raw }, "Job paused")}><Pause size={13} /> Pause</button>}
                    <button className="btn danger" disabled={busy} onClick={async () => { if (await confirm({ title: "Delete this job?", body: <div className="mono">{cur.command}</div>, confirmLabel: "Delete job", danger: true }) && await change(cur, { type: "delete", raw: cur.raw }, "Job deleted")) setSel(null); }}><Trash2 size={13} /> Delete</button>
                  </> : (
                    <button className="btn" onClick={() => cur.file && openFile(sid, cur.file)}><FileText size={13} /> Edit {cur.file?.split("/").pop()}</button>
                  )}
                </div>
                <dl className="kv">
                  <dt>Command</dt><dd className="mono">{cur.command}</dd>
                  {cur.note && <><dt>Note</dt><dd>{cur.note}</dd></>}
                  <dt>Runs as</dt><dd className="mono">{cur.user}</dd>
                  <dt>Defined in</dt><dd className="mono">{cur.file ? `${cur.file}:${cur.line}` : `${cur.user}'s crontab, line ${cur.line}`}</dd>
                  <dt>Next runs</dt><dd>{cur.nextRuns.length ? cur.nextRuns.map((n) => <div key={n}>{clock(n)} <span className="faint">({when(n, r.serverNow)})</span></div>) : "—"}</dd>
                  <dt>Last run</dt><dd>{cur.lastRun ? clock(cur.lastRun.replace(/([+-]\d{2})(\d{2})$/, "$1:$2")) : <span className="faint">Not in the cron log of the last 3 days</span>}</dd>
                  <dt>Status</dt><dd><StateDot state={cur.paused ? "inactive" : "running"} label={cur.paused ? "Paused" : "Scheduled"} /></dd>
                </dl>
              </>
            )}
          </aside>
        </div>
      )}
      {editing && r && <JobDialog s={s} report={r} job={editing === "new" ? null : editing} onClose={() => setEditing(null)} onSaved={setData} />}
      {out && (
        <Modal title={out.title} wide onClose={() => setOut(null)} footer={<button className="btn primary" onClick={() => setOut(null)}>Close</button>}>
          <pre className="output" style={{ maxHeight: 420 }}>{[out.o.stdout, out.o.stderr].filter((x) => x.trim()).join("\n") || "The job printed nothing."}</pre>
        </Modal>
      )}
    </WithServicePane>
  );
}
