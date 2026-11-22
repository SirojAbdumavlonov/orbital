import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { Archive, Database, Download, Folder, Pencil, Play, Plus, RotateCcw, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { confirm, Modal } from "../components/ui";
import { api, asApiError, BackupJob, BackupStatus, DbKind, RestoreTo } from "../lib/api";
import { bytes, relativeTime } from "../lib/format";
import { runJob, useRunningJob } from "../lib/jobs";
import { SessionState, useStore } from "../store";

// Scheduled backups (Backups tab): jobs live on the server as a script and a
// cron line (orbital_core::backups), so they run whether or not Orbital is
// open. Backups stay on the server; download one to keep a copy elsewhere.

const DAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const ENGINES: { kind: DbKind; name: string; tool: string }[] = [
  { kind: "postgres", name: "PostgreSQL", tool: "postgresql" },
  { kind: "mysql", name: "MySQL / MariaDB", tool: "mysql" },
  { kind: "mongo", name: "MongoDB", tool: "mongodb" },
];
const pad = (n: number) => String(n).padStart(2, "0");

export function scheduleText(j: BackupJob): string {
  const t = `${pad(j.schedule.hour)}:${pad(j.schedule.minute)}`;
  return j.schedule.every === "day" ? `Every day at ${t}` : `${DAYS[j.schedule.weekday]}s at ${t}`;
}

export function sourceText(j: BackupJob): string {
  const s = j.source;
  return s.kind === "folder" ? s.path : `${ENGINES.find((e) => e.kind === s.engine)?.name ?? s.engine} database ${s.name}`;
}

/** An id for a new job from its name, not taken yet. */
export function newId(name: string, taken: string[]): string {
  const base = name.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 36) || "backup";
  let id = base;
  for (let i = 2; taken.includes(id); i++) id = `${base}-${i}`;
  return id;
}

export function BackupsView({ s }: { s: SessionState }) {
  const sid = s.info.id;
  const [jobs, setJobs] = useState<BackupStatus[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState<BackupJob | "new" | null>(null);
  const [restoring, setRestoring] = useState<{ job: BackupJob; file: string } | null>(null);
  const [open, setOpen] = useState<string | null>(null);
  const load = () => api.backupJobs(sid).then((j) => { setJobs(j); setError(null); }, (e) => setError(asApiError(e).message));
  useEffect(() => { void load(); }, [sid]);
  const toast = useStore((x) => x.toast);

  async function runNow(j: BackupJob) {
    try {
      await runJob((p) => api.runBackup(sid, j.id, p), { key: `backup:${sid}:${j.id}`, title: `Backing up ${j.name}`, sid, done: (f) => `${j.name} backed up: ${f.split("/").pop()}` });
    } catch { /* the job reports it */ }
    void load();
  }

  async function remove(j: BackupJob) {
    const ok = await confirm({
      title: `Delete the backup job “${j.name}”?`,
      body: <>It stops running. Its backups in <span className="mono">/var/backups/orbital/{j.id}</span> are kept; delete that folder yourself if you don't need them.</>,
      confirmLabel: "Delete job",
      danger: true,
    });
    if (!ok) return;
    try {
      await api.deleteBackupJob(sid, j.id, false);
      toast({ kind: "success", title: `Backup job “${j.name}” deleted`, body: "Its backups were kept." });
    } catch (e) {
      toast({ kind: "error", title: "Couldn't delete the job", body: asApiError(e).message });
    }
    void load();
  }

  async function download(file: string) {
    const name = file.split("/").pop()!;
    const local = await saveDialog({ defaultPath: name, title: `Download ${name}` });
    if (!local) return;
    try {
      const n = await api.download(sid, file, local);
      toast({ kind: "success", title: `Downloaded ${name}`, body: bytes(n) });
    } catch (e) {
      toast({ kind: "error", title: "Download failed", body: asApiError(e).message });
    }
  }

  return (
    <div className="view">
      <div className="toolbar view-toolbar">
        <span className="faint" style={{ fontSize: 12.5 }}>Runs on the server on schedule, even when Orbital is closed. Backups are kept in <span className="mono">/var/backups/orbital</span>.</span>
        <span className="spacer" />
        <button className="btn primary" onClick={() => setEditing("new")}><Plus size={13} /> New backup</button>
      </div>
      <div className="scroll">
        {error && <div className="alert" style={{ margin: 16 }}>{error}</div>}
        {jobs && jobs.length === 0 && (
          <div className="empty">
            <div>
              <Archive size={28} />
              <h3>No scheduled backups yet</h3>
              <p>Back up a folder (a site, uploads, configs) or a database every day or week, and keep the last few.</p>
              <button className="btn primary" onClick={() => setEditing("new")}><Plus size={13} /> New backup</button>
            </div>
          </div>
        )}
        <div className="backup-list">
          {jobs?.map((st) => {
            const j = st.job;
            const Icon = j.source.kind === "folder" ? Folder : Database;
            return (
              <div key={j.id} className="backup-job">
                <div className="backup-job-head">
                  <Icon size={16} className="faint" />
                  <div className="backup-job-text">
                    <b>{j.name}</b>
                    <span className="faint">{sourceText(j)} · {scheduleText(j)} · keeps {j.keep}</span>
                    <span className={st.lastError ? "backup-bad" : "faint"}>
                      {!st.scheduled ? "Not scheduled (its cron line is missing); save it again. " : ""}
                      {st.lastRun ? `Last run ${relativeTime(st.lastRun)}${st.lastError ? `: failed: ${st.lastError}` : ""}` : "Hasn't run yet"}
                    </span>
                  </div>
                  <span className="spacer" />
                  <RunButton sid={sid} j={j} onRun={() => runNow(j)} />
                  <button className="icon-btn" aria-label={`Change ${j.name}`} title="Change" onClick={() => setEditing(j)}><Pencil size={14} /></button>
                  <button className="icon-btn" aria-label={`Delete ${j.name}`} title="Delete the job" onClick={() => remove(j)}><Trash2 size={14} /></button>
                </div>
                {st.files.length > 0 && (
                  <button className="backup-files-toggle" onClick={() => setOpen(open === j.id ? null : j.id)}>
                    {open === j.id ? "Hide" : "Show"} {st.files.length} backup{st.files.length === 1 ? "" : "s"} · newest {relativeTime(st.files[0].at)}, {bytes(st.files[0].bytes)}
                  </button>
                )}
                {open === j.id && (
                  <table className="table backup-files">
                    <tbody>
                      {st.files.map((f) => (
                        <tr key={f.path}>
                          <td className="mono">{f.path.split("/").pop()}</td>
                          <td className="faint nowrap">{relativeTime(f.at)}</td>
                          <td className="num">{bytes(f.bytes)}</td>
                          <td className="nowrap" style={{ textAlign: "right" }}>
                            <button className="btn ghost" onClick={() => download(f.path)}><Download size={13} /> Download</button>
                            <button className="btn ghost" onClick={() => setRestoring({ job: j, file: f.path })}><RotateCcw size={13} /> Restore…</button>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                )}
              </div>
            );
          })}
        </div>
      </div>
      {editing && <JobDialog s={s} job={editing === "new" ? null : editing} taken={(jobs ?? []).map((x) => x.job.id)} onClose={() => setEditing(null)} onSaved={() => { setEditing(null); void load(); }} />}
      {restoring && <RestoreDialog sid={sid} job={restoring.job} file={restoring.file} onClose={() => setRestoring(null)} />}
    </div>
  );
}

function RunButton({ sid, j, onRun }: { sid: string; j: BackupJob; onRun: () => void }) {
  const running = useRunningJob(`backup:${sid}:${j.id}`);
  return (
    <button className="btn" disabled={!!running} onClick={onRun} title="Back up now (as the schedule would)">
      <Play size={13} className={running ? "spin" : ""} /> {running ? "Backing up…" : "Back up now"}
    </button>
  );
}

function JobDialog({ s, job, taken, onClose, onSaved }: { s: SessionState; job: BackupJob | null; taken: string[]; onClose: () => void; onSaved: () => void }) {
  const sid = s.info.id;
  const engines = ENGINES.filter((e) => s.tools?.some((t) => t.id === e.tool && t.onHost !== false));
  const [name, setName] = useState(job?.name ?? "");
  const [kind, setKind] = useState<"folder" | "database">(job?.source.kind ?? "folder");
  const [path, setPath] = useState(job?.source.kind === "folder" ? job.source.path : "/var/www/");
  const [engine, setEngine] = useState<DbKind>(job?.source.kind === "database" ? job.source.engine : engines[0]?.kind ?? "postgres");
  const [db, setDb] = useState(job?.source.kind === "database" ? job.source.name : "");
  const [dbs, setDbs] = useState<string[]>([]);
  const [every, setEvery] = useState<"day" | "week">(job?.schedule.every ?? "day");
  const [weekday, setWeekday] = useState(job?.schedule.weekday ?? 0);
  const [time, setTime] = useState(job ? `${pad(job.schedule.hour)}:${pad(job.schedule.minute)}` : "03:30");
  const [keep, setKeep] = useState(String(job?.keep ?? 7));
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  useEffect(() => {
    if (kind !== "database") return;
    api.listDatabases(sid, engine).then((r) => setDbs(r.databases.filter((d) => !d.system).map((d) => d.name)), () => setDbs([]));
  }, [kind, engine, sid]);
  const [hh, mm] = time.split(":").map((x) => Number(x));
  const save = async () => {
    const j: BackupJob = {
      id: job?.id ?? newId(name, taken),
      name: name.trim() || (kind === "folder" ? path.split("/").filter(Boolean).pop() ?? "Folder" : db),
      source: kind === "folder" ? { kind, path: path.trim() } : { kind, engine, name: db.trim() },
      schedule: { every, weekday, hour: hh || 0, minute: mm || 0 },
      keep: Number(keep) || 0,
    };
    setBusy(true);
    setProblem(null);
    try {
      await runJob((p) => api.saveBackupJob(sid, j, p), { key: `backup-save:${sid}`, title: `Saving backup “${j.name}”`, sid, quietHere: true, done: () => `Backup “${j.name}” scheduled: ${scheduleText(j)}` });
      onSaved();
    } catch (e) {
      setProblem(asApiError(e).message.replace(/^Invalid input: /, ""));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Modal
      title={job ? `Change “${job.name}”` : "New scheduled backup"}
      subtitle="Saved on the server as a cron job; it runs as root at the time you choose (the server's clock)."
      keepOnOutsideClick
      onClose={onClose}
      footer={<><button className="btn" onClick={onClose}>Cancel</button><button className="btn primary" disabled={busy} onClick={save}>{busy ? "Saving…" : job ? "Save" : "Schedule backup"}</button></>}
    >
      <div className="route-form">
        <label className="field"><span>Name</span><input className="input" value={name} placeholder="Shop uploads" onChange={(e) => setName(e.target.value)} autoFocus /></label>
        <div className="field">
          <span>What to back up</span>
          <div className="segmented" style={{ alignSelf: "flex-start" }}>
            <button className={kind === "folder" ? "on" : ""} onClick={() => setKind("folder")}>A folder</button>
            <button className={kind === "database" ? "on" : ""} onClick={() => setKind("database")}>A database</button>
          </div>
        </div>
        {kind === "folder" ? (
          <label className="field"><span>Folder</span><input className="input mono" value={path} onChange={(e) => setPath(e.target.value)} placeholder="/var/www/example.com" /><span className="hint">Saved as a .tar.gz with everything inside it.</span></label>
        ) : (
          <div className="grid2">
            <label className="field">
              <span>Database server</span>
              <select className="input" value={engine} onChange={(e) => setEngine(e.target.value as DbKind)}>
                {(engines.length ? engines : ENGINES).map((e) => <option key={e.kind} value={e.kind}>{e.name}</option>)}
              </select>
            </label>
            <label className="field">
              <span>Database</span>
              <input className="input mono" list="backup-dbs" value={db} onChange={(e) => setDb(e.target.value)} placeholder="shop" />
              <datalist id="backup-dbs">{dbs.map((d) => <option key={d} value={d} />)}</datalist>
            </label>
          </div>
        )}
        <div className="grid2">
          <div className="field">
            <span>When</span>
            <div className="input-suffix">
              <select className="input" value={every === "day" ? "day" : String(weekday)} onChange={(e) => { if (e.target.value === "day") setEvery("day"); else { setEvery("week"); setWeekday(Number(e.target.value)); } }}>
                <option value="day">Every day</option>
                {DAYS.map((d, i) => <option key={d} value={i}>Every {d}</option>)}
              </select>
              <span>at</span>
              <input className="input mono" type="time" style={{ maxWidth: 110 }} value={time} onChange={(e) => setTime(e.target.value)} />
            </div>
          </div>
          <label className="field">
            <span>Keep the last</span>
            <div className="input-suffix"><input className="input mono" style={{ maxWidth: 80 }} inputMode="numeric" value={keep} onChange={(e) => setKeep(e.target.value.replace(/\D/g, "").slice(0, 2))} /><span>backups; older ones are deleted</span></div>
          </label>
        </div>
        {problem && <div className="alert">{problem}</div>}
      </div>
    </Modal>
  );
}

function RestoreDialog({ sid, job, file, onClose }: { sid: string; job: BackupJob; file: string; onClose: () => void }) {
  const folder = job.source.kind === "folder";
  const [to, setTo] = useState<RestoreTo>(folder ? "copy" : "original");
  const [typed, setTyped] = useState("");
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const word = job.source.kind === "folder" ? job.source.path.split("/").pop()! : job.source.name;
  const dangerous = to === "original";
  const go = async () => {
    setBusy(true);
    setProblem(null);
    try {
      await runJob((p) => api.restoreScheduledBackup(sid, job.id, file, to, p), { key: `backup-restore:${sid}`, title: `Restoring ${file.split("/").pop()}`, sid, done: (where) => `Restored to ${where}` });
      onClose();
    } catch (e) {
      setProblem(asApiError(e).message);
    } finally {
      setBusy(false);
    }
  };
  return (
    <Modal
      title={`Restore ${file.split("/").pop()}`}
      keepOnOutsideClick
      onClose={onClose}
      footer={<><button className="btn" onClick={onClose}>Cancel</button><button className={`btn ${dangerous ? "danger" : "primary"}`} disabled={busy || (dangerous && typed.trim() !== word)} onClick={go}>{busy ? "Restoring…" : "Restore"}</button></>}
    >
      <div className="route-form">
        {folder ? (
          <>
            <label className="check"><input type="radio" checked={to === "copy"} onChange={() => setTo("copy")} /> Into a new folder next to <span className="mono">{(job.source as { path: string }).path}</span> (nothing is overwritten)</label>
            <label className="check"><input type="radio" checked={to === "original"} onChange={() => setTo("original")} /> Over the original folder (its files are replaced by the backup's)</label>
          </>
        ) : (
          <p>The database <span className="mono">{word}</span> gets the backup's contents: what changed since then is lost.</p>
        )}
        {dangerous && (
          <label className="field">
            <span>Type <b className="mono">{word}</b> to confirm</span>
            <input className="input mono" value={typed} onChange={(e) => setTyped(e.target.value)} autoFocus />
          </label>
        )}
        {problem && <div className="alert" style={{ whiteSpace: "pre-wrap" }}>{problem}</div>}
      </div>
    </Modal>
  );
}
