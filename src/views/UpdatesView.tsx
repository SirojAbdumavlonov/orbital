import { Download, Power, RefreshCw, ShieldAlert } from "lucide-react";
import { useMemo, useState } from "react";
import { JobProgress } from "../components/JobProgress";
import { confirm, Modal } from "../components/ui";
import { api, asApiError, ExecOutput } from "../lib/api";
import { relativeTime } from "../lib/format";
import { runJob, useRunningJob } from "../lib/jobs";
import { useRemote } from "../lib/remote";
import { SessionState, useStore } from "../store";
import { useKept } from "../lib/kept";

/** System updates: what can be upgraded (security first), install them, reboot when needed. */
export function UpdatesView({ s }: { s: SessionState }) {
  const sid = s.info.id;
  const { data: r, error, loading, reload } = useRemote(() => api.updates(sid), [sid]);
  const job = useRunningJob(`updates:${sid}`);
  const [out, setOut] = useState<{ title: string; o: ExecOutput } | null>(null);
  const [q, setQ] = useKept(`${s.info.id}:updates:q`, "");
  const security = r?.packages.filter((p) => p.security).length ?? 0;
  const shown = useMemo(() => (r?.packages ?? []).filter((p) => !q || p.name.includes(q.toLowerCase())), [r, q]);

  async function refresh() {
    try {
      await runJob((p) => api.refreshUpdates(sid, p), { key: `updates:${sid}`, title: "Checking for updates", sid, done: (x) => (x.packages.length ? `${x.packages.length} updates available` : "Everything is up to date") });
    } catch {
      // The job reports the error.
    }
    reload();
  }

  async function install() {
    const n = r?.packages.length ?? 0;
    const ok = await confirm({
      title: `Install ${n} ${n === 1 ? "update" : "updates"}?`,
      body: "Services whose packages are updated may restart briefly. Configuration files you changed are kept. You can keep working; Orbital tells you when it's done.",
      confirmLabel: "Install updates",
    });
    if (!ok) return;
    try {
      const o = await runJob((p) => api.installUpdates(sid, p), { key: `updates:${sid}`, title: "Installing updates", sid, done: (x) => (x.exitCode === 0 ? "Updates installed" : { failed: "The package manager reported an error" }) });
      setOut({ title: o.exitCode === 0 ? "Updates installed" : "Updates failed", o });
    } catch {
      // The job reports the error.
    }
    reload();
  }

  async function reboot() {
    const ok = await confirm({
      title: `Reboot ${s.profile.name}?`,
      body: "Everything on the server stops for a minute or two: websites, databases and your terminals here. Orbital disconnects; connect again once it's back.",
      confirmLabel: "Reboot now",
      danger: true,
      requireText: s.profile.name,
    });
    if (!ok) return;
    try {
      await api.rebootServer(sid);
      useStore.getState().toast({ kind: "info", title: `${s.profile.name} is rebooting`, body: "Connect again in a minute or two." });
      await useStore.getState().disconnect(sid);
    } catch (e) {
      useStore.getState().toast({ kind: "error", title: "Couldn't reboot", body: asApiError(e).message });
    }
  }

  return (
    <div className="view">
      <div className="toolbar">
        {r?.manager && <span className="faint">{r.manager}{r.refreshedAt ? ` · list from ${relativeTime(r.refreshedAt)}` : ""}</span>}
        {r && r.packages.length > 0 && <input className="input" style={{ height: 26, maxWidth: 220 }} placeholder="Filter packages" value={q} onChange={(e) => setQ(e.target.value)} />}
        <span className="spacer" />
        {r?.rebootRequired && <button className="btn danger" disabled={!!job} onClick={reboot}><Power size={13} /> Reboot…</button>}
        <button className="btn" disabled={!!job || !r?.manager} onClick={refresh} title="Ask the package mirrors for new versions (apt-get update)"><RefreshCw size={13} /> Check for updates</button>
        <button className="btn primary" disabled={!!job || !r?.packages.length} onClick={install}><Download size={13} /> Install {r?.packages.length ? r.packages.length : ""} updates</button>
        <button className="icon-btn" aria-label="Refresh" onClick={reload}><RefreshCw size={14} className={loading ? "spin" : ""} /></button>
      </div>
      {job && <div className="job-bar"><JobProgress job={job} /></div>}
      {error && <div className="empty"><div><h3>Couldn't read updates</h3><p>{error.message}</p><button className="btn" onClick={reload}>Try again</button></div></div>}
      {r && !r.manager && <div className="empty"><div><h3>No package manager Orbital knows</h3><p>Updates work with apt (Debian, Ubuntu) and dnf or yum (Fedora, RHEL, Rocky, Alma).</p></div></div>}
      {r?.rebootRequired && <div className="notice"><Power size={13} style={{ verticalAlign: -2, marginRight: 6 }} />A reboot is needed for installed updates (often a new kernel) to take effect.</div>}
      {r?.manager && r.packages.length === 0 && !job && (
        <div className="empty"><div><h3>Everything is up to date</h3><p>{r.refreshedAt ? `The package list is from ${relativeTime(r.refreshedAt)}. ` : ""}Check for updates to ask the mirrors again.</p></div></div>
      )}
      {r && r.packages.length > 0 && (
        <div className="scroll">
          {security > 0 && <div className="notice"><ShieldAlert size={13} style={{ verticalAlign: -2, marginRight: 6 }} />{security} of {r.packages.length} are security updates: they fix known holes, so install them soon.</div>}
          <table className="table">
            <thead><tr><th>Package</th><th>Installed</th><th>New version</th><th /></tr></thead>
            <tbody>
              {shown.map((p) => (
                <tr key={p.name}>
                  <td className="mono">{p.name}</td>
                  <td className="mono faint">{p.current ?? "—"}</td>
                  <td className="mono">{p.available}</td>
                  <td>{p.security && <span className="badge bad">security</span>}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {out && (
        <Modal title={out.title} wide onClose={() => setOut(null)} footer={<button className="btn primary" onClick={() => setOut(null)}>Close</button>}>
          <pre className="output" style={{ maxHeight: 420 }}>{[out.o.stdout, out.o.stderr].filter((x) => x.trim()).join("\n").split("\n").slice(-200).join("\n") || "Nothing was printed."}</pre>
        </Modal>
      )}
    </div>
  );
}
