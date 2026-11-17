import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { Camera, Database, Download, Eraser, Loader2, RefreshCw, Server } from "lucide-react";
import { ReactNode, useState } from "react";
import { confirm, LiveNumber, Modal } from "../components/ui";
import { api, asApiError, BackupFormat, DbInfo, DbKind, RedisAction } from "../lib/api";
import { runJob, useRunningJob } from "../lib/jobs";
import { JobProgress } from "../components/JobProgress";
import { bytes, duration } from "../lib/format";
import { useRemote } from "../lib/remote";
import { SessionState, useStore } from "../store";
import { ToolView } from "./ToolView";

const KIND: Record<string, DbKind> = { postgresql: "postgres", mysql: "mysql", mongodb: "mongo" };

/** A tool tab with the tool's own page plus its service controls and logs (the generic tool view). */
export function WithServicePane({ s, toolId, label, children }: { s: SessionState; toolId: string; label: string; children: ReactNode }) {
  const [pane, setPane] = useState<"main" | "service">("main");
  return (
    <div className="view">
      <div className="subtabs">
        <div className="segmented">
          <button className={pane === "main" ? "on" : ""} onClick={() => setPane("main")}><Database size={12} /> {label}</button>
          <button className={pane === "service" ? "on" : ""} onClick={() => setPane("service")}><Server size={12} /> Service and logs</button>
        </div>
      </div>
      {pane === "main" ? children : <ToolView s={s} toolId={toolId} />}
    </div>
  );
}

const stamp = () => new Date().toISOString().slice(0, 16).replace(/[-:T]/g, "").replace(/^(\d{8})/, "$1-");

function BackupDialog({ s, kind, db, onClose }: { s: SessionState; kind: DbKind; db: DbInfo; onClose: () => void }) {
  const [format, setFormat] = useState<BackupFormat>(kind === "mysql" ? "sqlGz" : "native");
  const key = `backup:${s.info.id}:${db.name}`;
  const job = useRunningJob(key);
  const busy = !!job;
  const ext = kind === "mongo" ? "archive.gz" : kind === "postgres" && format === "native" ? "dump" : "sql.gz";

  async function go() {
    const local = await saveDialog({ title: `Back up ${db.name}`, defaultPath: `${db.name}-${stamp()}.${ext}` });
    if (!local) return;
    try {
      await runJob((p) => api.backupDatabase(s.info.id, kind, db.name, format, local, p), {
        key,
        title: `Backup of ${db.name}`,
        sid: s.info.id,
        quietHere: false,
        done: (n) => `${db.name} backed up: ${bytes(n)} saved to ${local}`,
      });
      onClose();
    } catch {
      // The job reports the error.
    }
  }

  const restore = {
    postgres: format === "native" ? `pg_restore -d ${db.name} ${db.name}.dump` : `gunzip -c ${db.name}.sql.gz | psql ${db.name}`,
    mysql: `gunzip -c ${db.name}.sql.gz | mysql`,
    mongo: `mongorestore --gzip --archive=${db.name}.archive.gz`,
  }[kind];
  return (
    <Modal title={`Back up ${db.name}`} subtitle="Orbital dumps the database on the server, downloads the file to this computer, and deletes the copy on the server." onClose={onClose}
      footer={<><button className="btn" onClick={onClose}>{busy ? "Continue in the background" : "Cancel"}</button><button className="btn primary" disabled={busy} onClick={go}>{busy ? <><Loader2 size={13} className="spin" /> Backing up…</> : <><Download size={13} /> Choose where to save…</>}</button></>}>
      <div className="form">
        {job && <JobProgress job={job} />}
        {kind === "postgres" && (
          <div className="field"><span>Format</span>
            <div className="segmented">
              <button className={format === "native" ? "on" : ""} onClick={() => setFormat("native")}>PostgreSQL custom (.dump)</button>
              <button className={format === "sqlGz" ? "on" : ""} onClick={() => setFormat("sqlGz")}>SQL (.sql.gz)</button>
            </div>
            <small className="hint">{format === "native" ? "Compressed, and pg_restore can restore single tables from it." : "Plain SQL, readable and portable to other tools."}</small>
          </div>
        )}
        <dl className="kv" style={{ padding: 0 }}>
          <dt>Database</dt><dd className="mono">{db.name}</dd>
          <dt>Size on server</dt><dd>{db.sizeBytes != null ? bytes(db.sizeBytes) : "—"} <span className="faint">(the file is usually smaller, it is compressed)</span></dd>
          <dt>Restore with</dt><dd className="mono">{restore}</dd>
        </dl>
        <p className="faint" style={{ margin: 0, fontSize: 12 }}>
          {kind === "mysql" ? "Uses --single-transaction, so InnoDB tables stay consistent without locking the site." : kind === "postgres" ? "pg_dump takes a consistent snapshot; the database stays online." : "mongodump reads while the database stays online."} Large databases can take several minutes.
        </p>
      </div>
    </Modal>
  );
}

/** PostgreSQL, MySQL or MongoDB: databases with size and connections, and one-click backups to the Mac. */
export function DatabaseView({ s, toolId }: { s: SessionState; toolId: string }) {
  const kind = KIND[toolId];
  const tool = s.tools?.find((t) => t.id === toolId);
  const { data, error, loading, reload } = useRemote(() => api.listDatabases(s.info.id, kind), [s.info.id, kind]);
  const [showSystem, setShowSystem] = useState(false);
  const [backup, setBackup] = useState<DbInfo | null>(null);
  const list = (data?.databases ?? []).filter((d) => showSystem || !d.system);
  const total = (data?.databases ?? []).reduce((n, d) => n + (d.sizeBytes ?? 0), 0);
  const isSql = kind !== "mongo";
  const hasTables = list.some((d) => d.tables != null);

  return (
    <WithServicePane s={s} toolId={toolId} label="Databases">
      <div className="toolbar">
        {data?.version && <span className="badge">{tool?.name} {data.version.split(" ")[0]}</span>}
        {data && !data.error && <span className="faint">{data.databases.filter((d) => !d.system).length} databases · {bytes(total)}{data.connections != null ? ` · ${data.connections}${data.maxConnections ? ` of ${data.maxConnections}` : ""} connections` : ""}</span>}
        <span className="spacer" />
        <label className="check" style={{ alignItems: "center" }}><input type="checkbox" checked={showSystem} onChange={(e) => setShowSystem(e.target.checked)} /><span>Show system databases</span></label>
        <button className="icon-btn" aria-label="Refresh" onClick={reload}><RefreshCw size={14} className={loading ? "spin" : ""} /></button>
      </div>
      {error && <div className="empty"><div><h3>Couldn't read {tool?.name ?? "the database"}</h3><p>{error.message}</p><button className="btn" onClick={reload}>Try again</button></div></div>}
      {data?.error && (
        <div className="notice">
          <b>{tool?.name} didn't answer Orbital's admin login.</b>
          <pre className="output" style={{ margin: "8px 0" }}>{data.error}</pre>
          {kind === "postgres" && "Orbital runs psql as the postgres system user through sudo. Turn on sudo for this server, then reconnect."}
          {kind === "mysql" && "Orbital logs in as root over MySQL's socket, the Debian and Ubuntu default. If root needs a password, add it to /root/.my.cnf on the server ([client] password=…)."}
          {kind === "mongo" && "This MongoDB requires a login. Orbital can only read servers that allow localhost access without one."}
        </div>
      )}
      {data && !data.error && (
        <div className="scroll">
          <table className="table">
            <thead><tr><th>Database</th><th className="num">Size</th>{hasTables && <th className="num">{isSql ? "Tables" : "Collections"}</th>}{kind !== "mongo" && <th className="num">Connections</th>}<th style={{ width: 120 }} /></tr></thead>
            <tbody>
              {list.map((d) => (
                <tr key={d.name}>
                  <td className="mono"><b>{d.name}</b>{d.system && <span className="faint"> system</span>}</td>
                  <td className="num">{d.sizeBytes != null ? bytes(d.sizeBytes) : "—"}</td>
                  {hasTables && <td className="num muted">{d.tables ?? "—"}</td>}
                  {kind !== "mongo" && <td className="num muted">{d.connections ?? "—"}</td>}
                  <td style={{ textAlign: "right" }}><button className="btn" onClick={() => setBackup(d)}><Download size={13} /> Back up</button></td>
                </tr>
              ))}
            </tbody>
          </table>
          {list.length === 0 && <div className="empty" style={{ flex: "none" }}><p>No databases yet{showSystem ? "" : " besides the system ones"}.</p></div>}
          <p className="footnote">Backups are made on the server with {kind === "postgres" ? "pg_dump" : kind === "mysql" ? "mysqldump" : "mongodump"}, downloaded to this computer, and removed from the server. Each one is recorded in the activity log.</p>
        </div>
      )}
      {backup && <BackupDialog s={s} kind={kind} db={backup} onClose={() => setBackup(null)} />}
    </WithServicePane>
  );
}

/** Redis: memory, clients and keys per database; snapshot and flush. */
export function RedisView({ s }: { s: SessionState }) {
  const toast = useStore((x) => x.toast);
  const { data: r, setData, error, loading, reload } = useRemote(() => api.redisInfo(s.info.id), [s.info.id]);
  const [busy, setBusy] = useState(false);

  async function act(action: RedisAction, what: string, danger?: { title: string; body: string; type: string }) {
    if (danger) {
      const ok = await confirm({ title: danger.title, body: danger.body, confirmLabel: what, danger: true, requireText: danger.type });
      if (!ok) return;
    }
    setBusy(true);
    try {
      setData(await api.redisAction(s.info.id, action));
      toast({ kind: "success", title: `${what}: done` });
    } catch (e) {
      toast({ kind: "error", title: `${what} failed`, body: asApiError(e).message });
    } finally {
      setBusy(false);
    }
  }

  const keys = r?.keyspace.reduce((n, k) => n + k.keys, 0) ?? 0;
  const hitRate = r?.hits != null && r.misses != null && r.hits + r.misses > 0 ? (r.hits / (r.hits + r.misses)) * 100 : null;
  return (
    <WithServicePane s={s} toolId="redis" label="Redis">
      <div className="toolbar">
        {r?.version && <span className="badge">Redis {r.version}{r.role ? ` · ${r.role}` : ""}</span>}
        <span className="spacer" />
        {r && !r.error && <button className="btn" disabled={busy} onClick={() => act("snapshot", "Snapshot")} title="Write the data to disk now (BGSAVE)"><Camera size={13} /> Save snapshot</button>}
        {r && !r.error && <button className="btn danger" disabled={busy || keys === 0} onClick={() => act("flushAll", "Flush everything", { title: "Delete every key in every database?", body: `All ${keys} keys are deleted at once. Caches rebuild themselves, but sessions, queues and anything stored only in Redis are lost.`, type: "flush all" })}><Eraser size={13} /> Flush all</button>}
        <button className="icon-btn" aria-label="Refresh" onClick={reload}><RefreshCw size={14} className={loading ? "spin" : ""} /></button>
      </div>
      {error && <div className="empty"><div><h3>Couldn't read Redis</h3><p>{error.message}</p><button className="btn" onClick={reload}>Try again</button></div></div>}
      {r?.error && <div className="notice"><b>Redis didn't answer.</b><pre className="output" style={{ marginTop: 8 }}>{r.error}</pre>Orbital uses the password from redis.conf when it sets one, which needs sudo for this server.</div>}
      {r && !r.error && (
        <div className="scroll">
          <div className="stat-row">
            <div className="stat"><small>Memory used</small><b>{bytes(r.usedMemory)}</b><span className="faint">{r.maxMemory ? `of ${bytes(r.maxMemory)} · ${r.evictionPolicy}` : "no limit set"}</span></div>
            <div className="stat"><small>Keys</small><b><LiveNumber value={keys} /></b><span className="faint">in {r.keyspace.length} databases</span></div>
            <div className="stat"><small>Clients</small><b><LiveNumber value={r.connectedClients} /></b><span className="faint">connected now</span></div>
            <div className="stat"><small>Cache hit rate</small><b>{hitRate != null ? <LiveNumber value={hitRate} decimals={1} suffix="%" /> : "—"}</b><span className="faint">since start</span></div>
            <div className="stat"><small>Up for</small><b>{r.uptimeSecs != null ? duration(r.uptimeSecs) : "—"}</b><span className="faint">{r.lastSave ? `last snapshot ${new Date(r.lastSave * 1000).toLocaleString()}${r.lastSaveOk === false ? " (failed)" : ""}` : "no snapshot yet"}{r.aofEnabled ? " · AOF on" : ""}</span></div>
          </div>
          <table className="table">
            <thead><tr><th>Database</th><th className="num">Keys</th><th className="num">With expiry</th><th style={{ width: 120 }} /></tr></thead>
            <tbody>
              {r.keyspace.map((k) => (
                <tr key={k.db}>
                  <td className="mono">db{k.db}</td>
                  <td className="num">{k.keys.toLocaleString()}</td>
                  <td className="num muted">{k.expires.toLocaleString()}</td>
                  <td style={{ textAlign: "right" }}>
                    <button className="btn danger" disabled={busy} onClick={() => act({ flushDb: k.db }, `Flush db${k.db}`, { title: `Delete every key in db${k.db}?`, body: `${k.keys} keys are deleted. Other databases are not touched.`, type: `db${k.db}` })}><Eraser size={13} /> Flush</button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          {r.keyspace.length === 0 && <div className="empty" style={{ flex: "none" }}><p>Redis is empty.</p></div>}
        </div>
      )}
    </WithServicePane>
  );
}
