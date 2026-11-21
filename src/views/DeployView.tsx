import { AlertTriangle, CheckCircle2, GitBranch, History, Loader2, Pencil, Plus, RefreshCw, Rocket, RotateCcw, Trash2, XCircle } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { confirm, Modal } from "../components/ui";
import { api, App, AppInput, asApiError, Checkout, DeployRecord, RestartKind } from "../lib/api";
import { SessionState, useStore } from "../store";
import { useKept } from "../lib/kept";

const blank = (): AppInput => ({ name: "", path: "/srv/", branch: "main", build: "", restartKind: "none", restartTarget: null });

const BUILD_PRESETS: [string, string][] = [
  ["Node.js", "npm ci\nnpm run build"],
  ["PHP / Laravel", "composer install --no-dev --optimize-autoloader\nphp artisan migrate --force\nphp artisan config:cache"],
  ["Python", ".venv/bin/pip install -r requirements.txt\n.venv/bin/python manage.py migrate --noinput"],
  ["Docker Compose", "docker compose build"],
];

function AppDialog({ s, app, onClose, onSaved }: { s: SessionState; app: App | null; onClose: () => void; onSaved: (a: App) => void }) {
  const toast = useStore((x) => x.toast);
  const [f, setF] = useState<AppInput>(app ? { name: app.name, path: app.path, branch: app.branch, build: app.build, restartKind: app.restartKind, restartTarget: app.restartTarget } : blank());
  const [units, setUnits] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const set = (p: Partial<AppInput>) => setF((x) => ({ ...x, ...p }));
  useEffect(() => {
    if (s.info.hasSystemd) api.listServices(s.info.id).then((l) => setUnits(l.filter((u) => u.unit.endsWith(".service")).map((u) => u.unit))).catch(() => {});
  }, [s.info.id]);

  async function save() {
    setBusy(true);
    try {
      onSaved(await api.saveApp(s.profile.id, app?.id ?? null, f));
      onClose();
    } catch (e) {
      toast({ kind: "error", title: "Couldn't save the app", body: asApiError(e).message });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal title={app ? `Edit ${app.name}` : "Add an app to deploy"} wide subtitle="An app is a git checkout on this server. Deploying fetches its branch, runs the build commands and restarts it."
      onClose={busy ? undefined : onClose}
      footer={<><button className="btn" onClick={onClose}>Cancel</button><button className="btn primary" disabled={busy || !f.name.trim() || !f.path.startsWith("/")} onClick={save}>{app ? "Save" : "Add app"}</button></>}>
      <div className="form">
        <div className="row2" style={{ gridTemplateColumns: "1fr 2fr 1fr" }}>
          <label className="field"><span>Name</span><input className="input" autoFocus value={f.name} onChange={(e) => set({ name: e.target.value })} placeholder="Shop" /></label>
          <label className="field"><span>Folder on the server (a git checkout)</span><input className="input mono" value={f.path} onChange={(e) => set({ path: e.target.value })} placeholder="/srv/shop" /></label>
          <label className="field"><span>Branch</span><input className="input mono" value={f.branch} onChange={(e) => set({ branch: e.target.value })} /></label>
        </div>
        <label className="field"><span>Build commands <span className="faint">(one per line, run in the folder; leave empty if there's nothing to build)</span></span>
          <div className="chips">{BUILD_PRESETS.map(([l, b]) => <button key={l} className="chip" onClick={() => set({ build: b })}>{l}</button>)}</div>
          <textarea className="input mono" rows={4} value={f.build} onChange={(e) => set({ build: e.target.value })} placeholder={"npm ci\nnpm run build"} />
        </label>
        <div className="field"><span>Then restart it with</span>
          <div className="segmented">
            {([["none", "Nothing"], ["service", "A service"], ["compose", "docker compose up -d"], ["command", "A command"]] as [RestartKind, string][]).map(([k, l]) => (
              <button key={k} className={f.restartKind === k ? "on" : ""} onClick={() => set({ restartKind: k, restartTarget: k === "service" ? units[0] ?? null : null })}>{l}</button>
            ))}
          </div>
          {f.restartKind === "service" && (
            <select className="input mono" value={f.restartTarget ?? ""} onChange={(e) => set({ restartTarget: e.target.value })}>
              {units.length === 0 && <option value="">No services found</option>}
              {units.map((u) => <option key={u}>{u}</option>)}
            </select>
          )}
          {f.restartKind === "command" && <input className="input mono" value={f.restartTarget ?? ""} onChange={(e) => set({ restartTarget: e.target.value })} placeholder="pm2 reload ecosystem.config.js" />}
          <small className="hint">{f.restartKind === "service" ? "Restarted like in the Services tab, with its configuration check first." : f.restartKind === "compose" ? "Recreates the containers whose image or settings changed." : ""}</small>
        </div>
      </div>
    </Modal>
  );
}

interface LiveStep { name: string; state: "running" | "ok" | "failed"; code?: number | null }

/** Ship code: pull the branch, build, restart; see what runs and roll back. */
export function DeployView({ s }: { s: SessionState }) {
  const toast = useStore((x) => x.toast);
  const sid = s.info.id;
  const [apps, setApps] = useState<App[] | null>(null);
  const [sel, setSel] = useKept<string | null>(`${s.info.id}:deploy:sel`, null);
  const [editing, setEditing] = useState<App | "new" | null>(null);
  const [checkout, setCheckout] = useState<Checkout | null>(null);
  const [checkoutError, setCheckoutError] = useState<string | null>(null);
  const [history, setHistory] = useState<DeployRecord[]>([]);
  const [steps, setSteps] = useState<LiveStep[]>([]);
  const [output, setOutput] = useState("");
  const [running, setRunning] = useState(false);
  const [logFor, setLogFor] = useState<{ title: string; text: string } | null>(null);
  const out = useRef<HTMLPreElement>(null);

  const loadApps = () => api.listApps(s.profile.id).then((a) => { setApps(a); setSel((cur) => cur ?? a[0]?.id ?? null); }).catch(() => setApps([]));
  useEffect(() => { loadApps(); }, [s.profile.id]);
  const app = apps?.find((a) => a.id === sel) ?? null;

  const loadApp = async () => {
    if (!app) return;
    setCheckout(null);
    setCheckoutError(null);
    api.appCheckout(sid, app.id).then(setCheckout).catch((e) => setCheckoutError(asApiError(e).message));
    api.deployHistory(app.id).then(setHistory).catch(() => setHistory([]));
  };
  useEffect(() => { setSteps([]); setOutput(""); loadApp(); }, [app?.id]);
  useEffect(() => { if (out.current) out.current.scrollTop = out.current.scrollHeight; }, [output]);

  async function run(rollbackTo: string | null) {
    if (!app) return;
    if (rollbackTo) {
      const ok = await confirm({ title: `Roll ${app.name} back to ${rollbackTo.slice(0, 7)}?`, body: "That commit is checked out, built and restarted. The next normal deploy brings the branch back.", confirmLabel: "Roll back" });
      if (!ok) return;
    }
    setRunning(true);
    setSteps([]);
    setOutput("");
    try {
      const rec = await api.runDeploy(sid, app.id, rollbackTo, (ev) => {
        if (ev.type === "step") setSteps((x) => [...x, { name: ev.name, state: "running" }]);
        else if (ev.type === "output") setOutput((o) => (o + ev.text).slice(-200_000));
        else if (ev.type === "stepEnd") setSteps((x) => x.map((st, i) => (i === x.length - 1 ? { ...st, state: ev.ok ? "ok" : "failed", code: ev.code } : st)));
      });
      toast(rec.outcome === "success" ? { kind: "success", title: `${app.name}: ${rec.summary ?? "deployed"}` } : { kind: "error", title: `${app.name}: deploy failed`, body: rec.summary ?? undefined });
    } catch (e) {
      const err = asApiError(e);
      toast({ kind: "error", title: err.kind === "PreflightFailed" ? "Not deployed: there are uncommitted changes on the server" : "Couldn't deploy", body: err.data?.output ?? err.message });
    } finally {
      setRunning(false);
      loadApp();
    }
  }

  async function remove(a: App) {
    if (!(await confirm({ title: `Remove ${a.name} from Deploy?`, body: "Only Orbital's settings for it are removed. Nothing on the server changes.", confirmLabel: "Remove", danger: true }))) return;
    await api.deleteApp(a.id).catch(() => {});
    setSel(null);
    loadApps();
  }

  const current = checkout?.head ?? null;
  return (
    <div className="view">
      <div className="deploy">
        <aside className="deploy-apps">
          <div className="section-title" style={{ display: "flex", alignItems: "center" }}>Apps<span className="spacer" style={{ flex: 1 }} />
            <button className="icon-btn" aria-label="Add an app" title="Add an app" onClick={() => setEditing("new")}><Plus size={14} /></button>
          </div>
          {apps?.map((a) => (
            <button key={a.id} className={`deploy-app ${a.id === sel ? "on" : ""}`} onClick={() => setSel(a.id)}>
              <b>{a.name}</b><span className="mono faint">{a.path}</span>
            </button>
          ))}
          {apps && apps.length === 0 && <p className="faint" style={{ padding: "0 12px", fontSize: 12 }}>No apps yet.</p>}
        </aside>
        {!app ? (
          <div className="empty"><div>
            <Rocket size={28} className="faint" />
            <h3>Deploy your code in one click</h3>
            <p>Add an app that lives in a git checkout on this server. Orbital fetches its branch, runs your build commands and restarts it, shows every step live, and keeps a history you can roll back from.</p>
            <button className="btn primary" onClick={() => setEditing("new")}><Plus size={13} /> Add an app</button>
          </div></div>
        ) : (
          <div className="scroll">
            <div className="deploy-head">
              <div>
                <h2>{app.name}</h2>
                <div className="faint mono" style={{ fontSize: 12 }}><GitBranch size={12} style={{ verticalAlign: -2 }} /> {app.branch} · {app.path}{checkout?.remote ? ` · ${checkout.remote}` : ""}</div>
              </div>
              <span className="spacer" />
              <button className="icon-btn" aria-label="Refresh" onClick={loadApp}><RefreshCw size={14} /></button>
              <button className="btn" onClick={() => setEditing(app)}><Pencil size={13} /> Edit</button>
              <button className="icon-btn" aria-label="Remove app" title="Remove from Deploy" onClick={() => remove(app)}><Trash2 size={14} /></button>
              <button className="btn primary big" disabled={running || !!checkoutError} onClick={() => run(null)}>{running ? <Loader2 size={14} className="spin" /> : <Rocket size={14} />} {running ? "Deploying…" : "Deploy"}</button>
            </div>
            {checkoutError && <div className="alert" style={{ margin: "0 16px 12px" }}>{checkoutError}</div>}
            {checkout && checkout.dirty.length > 0 && (
              <div className="notice warn-notice"><AlertTriangle size={14} /> These files were changed on the server and aren't committed, so deploying is blocked until they are committed or reverted: <span className="mono">{checkout.dirty.join(", ")}</span></div>
            )}
            {checkout && (
              <dl className="kv" style={{ padding: "0 16px 8px" }}>
                <dt>Running now</dt><dd className="mono">{current?.slice(0, 7) ?? "—"} {(() => { const c = checkout.recent.find((x) => x.id === checkout.head); return c && <span className="faint" style={{ fontFamily: "var(--sans)" }}>{c.subject} · {c.author}, {c.when}</span>; })()}</dd>
                <dt>Checked out</dt><dd className="mono">{checkout.branch === "HEAD" ? "a specific commit (rolled back)" : checkout.branch}</dd>
              </dl>
            )}

            {(steps.length > 0 || running) && (
              <div className="deploy-run">
                <ol className="deploy-steps">
                  {steps.map((st, i) => (
                    <li key={i} className={st.state}>
                      {st.state === "running" ? <Loader2 size={13} className="spin" /> : st.state === "ok" ? <CheckCircle2 size={13} /> : <XCircle size={13} />}
                      <span className="mono">{st.name}</span>
                      {st.state === "failed" && st.code != null && <span className="faint">exit {st.code}</span>}
                    </li>
                  ))}
                </ol>
                <pre className="deploy-output" ref={out}>{output || "Starting…"}</pre>
              </div>
            )}

            {checkout && checkout.recent.length > 0 && <>
              <div className="section-title">Recent commits</div>
              <table className="table">
                <tbody>
                  {checkout.recent.map((c) => (
                    <tr key={c.id}>
                      <td className="mono" style={{ width: 80 }}>{c.short}</td>
                      <td>{c.subject}<div className="faint" style={{ fontSize: 12 }}>{c.author}, {c.when}</div></td>
                      <td className="nowrap" style={{ textAlign: "right" }}>
                        {c.id === current ? <span className="badge ok">Running</span> : <button className="btn" disabled={running} onClick={() => run(c.id)}><RotateCcw size={13} /> Roll back to this</button>}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </>}

            <div className="section-title"><History size={12} style={{ verticalAlign: -1 }} /> History</div>
            <table className="table">
              <tbody>
                {history.map((h) => (
                  <tr key={h.id} className="click" onClick={async () => setLogFor({ title: `${h.kind === "rollback" ? "Rollback" : "Deploy"} on ${new Date(h.startedAt).toLocaleString()}`, text: await api.deployLog(h.id).catch(() => "") })} title="Show its output">
                    <td style={{ width: 24 }}>{h.outcome === "success" ? <CheckCircle2 size={14} color="var(--ok)" /> : h.outcome === "failure" ? <XCircle size={14} color="var(--bad)" /> : <Loader2 size={14} className="spin" />}</td>
                    <td className="nowrap">{new Date(h.startedAt).toLocaleString()}</td>
                    <td>{h.kind === "rollback" ? "Rollback" : "Deploy"}: {h.summary ?? ""}</td>
                    <td className="mono faint nowrap">{h.fromCommit?.slice(0, 7) ?? "?"} → {h.toCommit?.slice(0, 7) ?? "?"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            {history.length === 0 && <p className="faint" style={{ padding: "0 16px", fontSize: 12 }}>Not deployed from Orbital yet.</p>}
            <p className="footnote">A deploy only fast-forwards: it never discards commits or changes on the server. It stops at the first failing step; if that happens before the restart, the running app is untouched, though its folder already has the new code, so deploy again once it's fixed. Every deploy is in the activity log.</p>
          </div>
        )}
      </div>
      {editing && <AppDialog s={s} app={editing === "new" ? null : editing} onClose={() => setEditing(null)} onSaved={(a) => { loadApps(); setSel(a.id); }} />}
      {logFor && (
        <Modal title={logFor.title} wide onClose={() => setLogFor(null)} footer={<button className="btn primary" onClick={() => setLogFor(null)}>Close</button>}>
          <pre className="output" style={{ maxHeight: 460 }}>{logFor.text || "No output was kept."}</pre>
        </Modal>
      )}
    </div>
  );
}
