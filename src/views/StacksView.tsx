import { AlertTriangle, Check, Copy, ExternalLink, FileText, Globe, LayoutGrid, Layers, Play, Plus, RefreshCw, RotateCw, ScrollText, Square } from "lucide-react";
import { ReactNode, useEffect, useState } from "react";
import { confirm, Modal } from "../components/ui";
import { api, AppInstalled, AppOption, asApiError, DockerPlugins, Stack, StackAction } from "../lib/api";
import { runJob, useRunningJob } from "../lib/jobs";
import { useKept } from "../lib/kept";
import { openLocalPort } from "../lib/links";
import { withSafetyOverride } from "../lib/remote";
import { SessionState, useStore } from "../store";
import { forward } from "./TunnelsView";

// Docker Compose stacks: every Compose project on the server (deployed from
// anywhere) plus Orbital's own in /opt/stacks. Create one from a template
// (checked by `docker compose config` before it's kept), edit its file in the
// editor (safe save), deploy, pull and redeploy, restart, stop, read its logs.

const TEMPLATE = (name: string) => `# ${name}: a Docker Compose stack made in Orbital.
services:
  web:
    image: nginx:alpine
    restart: unless-stopped
    ports:
      - "127.0.0.1:8080:80"   # only this server; use a route in Nginx to publish it
`;

export function StacksView({ s }: { s: SessionState }) {
  const sid = s.info.id;
  const [list, setList] = useState<Stack[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [logs, setLogs] = useState<{ name: string; text: string } | null>(null);
  const { openFile } = useStore();
  const [mode, setMode] = useKept<"stacks" | "apps">(`${sid}:stacks:mode`, "stacks");
  const [plugins, setPlugins] = useState<DockerPlugins | null>(null);
  const hasDocker = !!s.info.dockerVersion || !!s.tools?.some((t) => t.id === "docker");
  const load = () => api.stacks(sid).then((l) => { setList(l); setError(null); }, (e) => setError(asApiError(e).message));
  useEffect(() => { if (hasDocker) { void load(); void api.dockerPlugins(sid).then(setPlugins, () => {}); } }, [sid, hasDocker]);
  const pluginsJob = useRunningJob(`docker-plugins:${sid}`);
  const installPlugins = async () => {
    try {
      setPlugins(await runJob((p) => api.installDockerPlugins(sid, p), { key: `docker-plugins:${sid}`, title: "Installing Docker Compose and Buildx", sid, done: (x) => (x.compose ? "Docker Compose is installed" : { failed: "Docker Compose is still missing" }) }));
      void load();
    } catch { /* the job reports it */ }
  };
  const pluginNotice = plugins && plugins.docker && !plugins.compose ? (
    <div className="notice plugin-notice">
      <AlertTriangle size={14} />
      <span>
        <b>Docker Compose isn't installed</b>, so stacks and apps can't run.{" "}
        {plugins.source === "docker-ce" ? "Docker came from Docker's own repository; " : plugins.source ? `Docker came from the ${plugins.source} package; ` : ""}
        Orbital installs the matching Compose{plugins.buildx ? "" : " and Buildx"} packages.
      </span>
      <button className="btn primary" disabled={!!pluginsJob} onClick={() => void installPlugins()}>{pluginsJob ? "Installing…" : "Install Docker Compose"}</button>
    </div>
  ) : null;

  const act = async (st: Stack, action: StackAction) => {
    const file = st.files[0];
    if (!file) return;
    if (action === "down") {
      const ok = await confirm({ title: `Stop ${st.name}?`, body: <>Its containers and networks are removed (<span className="mono">docker compose down</span>); volumes and the compose file are kept, so Deploy brings it back.</>, confirmLabel: "Stop stack", danger: true });
      if (!ok) return;
    }
    if (action === "logs") {
      setLogs({ name: st.name, text: "Reading…" });
      try {
        const o = await api.stackAction(sid, st.name, file, "logs");
        setLogs({ name: st.name, text: o.stdout || o.stderr || "No log lines yet." });
      } catch (e) {
        setLogs({ name: st.name, text: asApiError(e).message });
      }
      return;
    }
    const verb = { up: "Deploying", pullUp: "Pulling and redeploying", restart: "Restarting", down: "Stopping" }[action];
    try {
      const o = await runJob((p) => api.stackAction(sid, st.name, file, action, p), {
        key: `stack:${sid}:${st.name}`, title: `${verb} ${st.name}`, sid,
        done: (x) => (x.exitCode === 0 ? `${st.name}: done` : { failed: `${st.name}: docker compose failed` }),
      });
      if (o.exitCode !== 0) setLogs({ name: st.name, text: o.stdout || o.stderr });
    } catch { /* the job reports it */ }
    void load();
  };

  if (!hasDocker) {
    return <div className="empty"><div><Layers size={28} /><h3>Docker isn't on this server</h3><p>Install it from the + next to the server's tabs, then create stacks here.</p></div></div>;
  }
  const modes = (
    <div className="segmented" role="tablist">
      <button className={mode === "stacks" ? "on" : ""} onClick={() => setMode("stacks")}><Layers size={13} /> Stacks <span className="faint">{list?.length ?? ""}</span></button>
      <button className={mode === "apps" ? "on" : ""} onClick={() => setMode("apps")}><LayoutGrid size={13} /> Apps</button>
    </div>
  );
  if (mode === "apps") {
    return <AppsGallery s={s} stacks={list ?? []} modes={modes} notice={pluginNotice} onChanged={() => void load()} onShowStacks={() => setMode("stacks")} />;
  }
  return (
    <div className="view">
      <div className="toolbar view-toolbar">
        {modes}
        <span className="faint" style={{ fontSize: 12.5 }}>Compose projects on this server. New ones go to <span className="mono">/opt/stacks/&lt;name&gt;</span>.</span>
        <span className="spacer" />
        <button className="btn primary" onClick={() => setCreating(true)}><Plus size={13} /> New stack</button>
        <button className="icon-btn" aria-label="Refresh" onClick={() => void load()}><RefreshCw size={14} /></button>
      </div>
      <div className="scroll">
        {pluginNotice}
        {error && <div className="alert" style={{ margin: 16 }}>{error}</div>}
        {list && list.length === 0 && <div className="empty"><div><Layers size={28} /><h3>No Compose stacks yet</h3><p>Install a ready-made app from <button className="link-btn" onClick={() => setMode("apps")}>Apps</button>, create one from a template, or deploy a compose file you already have on the server.</p></div></div>}
        <div className="backup-list">
          {list?.map((st) => <StackRow key={st.name} sid={sid} st={st} onAct={(a) => act(st, a)} onEdit={() => st.files[0] && openFile(sid, st.files[0])} />)}
        </div>
      </div>
      {creating && <NewStackDialog sid={sid} taken={(list ?? []).map((x) => x.name)} onClose={() => setCreating(false)} onCreated={(file) => { setCreating(false); void load(); openFile(sid, file); }} />}
      {logs && (
        <Modal title={`Logs: ${logs.name}`} subtitle="The last 200 lines of each service." wide onClose={() => setLogs(null)}>
          <pre className="output stack-logs">{logs.text}</pre>
        </Modal>
      )}
    </div>
  );
}

function StackRow({ sid, st, onAct, onEdit }: { sid: string; st: Stack; onAct: (a: StackAction) => void; onEdit: () => void }) {
  const running = useRunningJob(`stack:${sid}:${st.name}`);
  return (
    <div className="backup-job">
      <div className="backup-job-head">
        <span className={`dot ${st.running ? "live" : ""}`} />
        <div className="backup-job-text">
          <b>{st.name}</b>
          <span className="faint">{st.status}{st.ours ? " · made in Orbital" : ""}</span>
          <span className="faint mono">{st.files.join(", ")}</span>
        </div>
        <span className="spacer" />
        <button className="btn" disabled={!!running} onClick={() => onAct(st.running ? "pullUp" : "up")} title={st.running ? "docker compose pull, then up -d" : "docker compose up -d"}>
          <Play size={13} className={running ? "spin" : ""} /> {running ? "Working…" : st.running ? "Pull & redeploy" : "Deploy"}
        </button>
        {st.running && <button className="icon-btn" aria-label={`Restart ${st.name}`} title="Restart its containers" disabled={!!running} onClick={() => onAct("restart")}><RotateCw size={14} /></button>}
        {st.running && <button className="icon-btn" aria-label={`Stop ${st.name}`} title="Stop (docker compose down)" disabled={!!running} onClick={() => onAct("down")}><Square size={14} /></button>}
        <button className="icon-btn" aria-label={`Logs of ${st.name}`} title="Logs" onClick={() => onAct("logs")}><ScrollText size={14} /></button>
        <button className="icon-btn" aria-label={`Edit ${st.name}`} title="Edit its compose file (safe save)" onClick={onEdit}><FileText size={14} /></button>
      </div>
    </div>
  );
}

function NewStackDialog({ sid, taken, onClose, onCreated }: { sid: string; taken: string[]; onClose: () => void; onCreated: (file: string) => void }) {
  const [name, setName] = useState("");
  const [content, setContent] = useState(TEMPLATE("my-stack"));
  const [deploy, setDeploy] = useState(true);
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const clean = name.trim().toLowerCase().replace(/[^a-z0-9_-]+/g, "-").replace(/^-+|-+$/g, "");
  const go = async () => {
    setBusy(true);
    setProblem(null);
    try {
      const out = await runJob((p) => api.createStack(sid, clean, content, deploy, p), {
        key: `stack-new:${sid}`, title: `Creating stack ${clean}`, sid,
        done: (o) => (!o || o.exitCode === 0 ? `Stack ${clean} ${deploy ? "is running" : "is saved"}` : { failed: `Stack ${clean} was saved, but deploying it failed` }),
      });
      if (out && out.exitCode !== 0) setProblem(`Saved, but docker compose up failed:\n${out.stdout || out.stderr}`);
      else onCreated(`/opt/stacks/${clean}/compose.yaml`);
    } catch (e) {
      const err = asApiError(e);
      setProblem(err.kind === "ValidationFailed" ? `docker compose rejected the file, so nothing was saved:\n${err.data?.output ?? err.message}` : err.message.replace(/^Invalid input: /, ""));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Modal title="New stack" subtitle="A compose.yaml in /opt/stacks/<name>, checked by docker compose before it's saved." wide keepOnOutsideClick onClose={onClose}
      footer={<><button className="btn" onClick={onClose}>Cancel</button><button className="btn primary" disabled={busy || !clean || taken.includes(clean)} onClick={go}>{busy ? "Working…" : deploy ? "Create and deploy" : "Create"}</button></>}>
      <div className="route-form">
        <label className="field">
          <span>Name</span>
          <input className="input mono" value={name} onChange={(e) => setName(e.target.value)} placeholder="shop" autoFocus />
          <span className="hint">{clean ? (taken.includes(clean) ? `${clean} already exists` : `/opt/stacks/${clean}/compose.yaml · project ${clean}`) : "Lowercase letters, digits, - and _"}</span>
        </label>
        <label className="field">
          <span>compose.yaml</span>
          <textarea className="input mono stack-yaml" rows={14} value={content} onChange={(e) => setContent(e.target.value)} spellCheck={false} />
        </label>
        <label className="check"><input type="checkbox" checked={deploy} onChange={(e) => setDeploy(e.target.checked)} /> Deploy it now (docker compose up -d)</label>
        {problem && <div className="alert" style={{ whiteSpace: "pre-wrap" }}>{problem}</div>}
      </div>
    </Modal>
  );
}

// ---------- Apps: ready-made stacks (apps.rs) ----------

/** The port each installed app was given (kept for this session, for Open). */
const portsKey = (sid: string) => `${sid}:apps:ports`;

function AppsGallery({ s, stacks, modes, notice, onChanged, onShowStacks }: { s: SessionState; stacks: Stack[]; modes: ReactNode; notice: ReactNode; onChanged: () => void; onShowStacks: () => void }) {
  const sid = s.info.id;
  const [apps, setApps] = useState<AppOption[] | null>(null);
  const [query, setQuery] = useKept(`${sid}:apps:query`, "");
  const [installing, setInstalling] = useState<AppOption | null>(null);
  const [done, setDone] = useState<AppInstalled | null>(null);
  const [ports, setPorts] = useKept<Record<string, number>>(portsKey(sid), {});
  useEffect(() => { void api.appOptions().then(setApps); }, []);
  const q = query.trim().toLowerCase();
  const shown = (apps ?? []).filter((a) => !q || `${a.name} ${a.category} ${a.hint}`.toLowerCase().includes(q));
  const stackOf = (a: AppOption) => stacks.find((x) => x.name === a.id || x.name.startsWith(`${a.id}-`));
  return (
    <div className="view">
      <div className="toolbar view-toolbar">
        {modes}
        <span className="faint" style={{ fontSize: 12.5 }}>Self-hosted apps as Compose stacks, private to the server (127.0.0.1) until you publish them.</span>
        <span className="spacer" />
        <input className="input" style={{ width: 200 }} placeholder="Search apps" value={query} onChange={(e) => setQuery(e.target.value)} />
      </div>
      <div className="scroll">
        {notice}
        <div className="app-grid">
          {shown.map((a) => {
            const st = stackOf(a);
            const port = st ? ports[st.name] ?? a.port : a.port;
            return (
              <div key={a.id} className="app-card">
                <div className="app-card-head">
                  <b>{a.name}</b>
                  <span className="badge">{a.category}</span>
                </div>
                <p>{a.hint}</p>
                <span className="faint app-card-meta">~{a.memoryMb >= 1024 ? `${(a.memoryMb / 1024).toFixed(0)} GB` : `${a.memoryMb} MB`} memory · <span className="mono">{a.images.join(", ")}</span></span>
                <div className="app-card-actions">
                  {st ? (
                    <>
                      <span className={`dot ${st.running ? "live" : ""}`} /> <span className="faint">{st.running ? "Running" : st.status}</span>
                      <span className="spacer" />
                      {st.running && <button className="btn" title={`Forward port ${port} to this computer and open it`} onClick={() => void openApp(s, port)}><ExternalLink size={13} /> Open</button>}
                      <button className="btn" onClick={onShowStacks}><Layers size={13} /> Manage</button>
                    </>
                  ) : (
                    <>
                      <span className="spacer" />
                      <button className="btn primary" onClick={() => setInstalling(a)}><Plus size={13} /> Install</button>
                    </>
                  )}
                </div>
              </div>
            );
          })}
        </div>
        {apps && shown.length === 0 && <div className="empty"><div><LayoutGrid size={28} /><h3>No app matches</h3><p>Anything with a compose file can still go in through New stack.</p></div></div>}
      </div>
      {installing && (
        <InstallAppDialog s={s} app={installing} taken={stacks.map((x) => x.name)} onClose={() => setInstalling(null)}
          onInstalled={(r) => { setInstalling(null); setPorts((p) => ({ ...p, [r.name]: r.port })); setDone(r); onChanged(); }} />
      )}
      {done && <InstalledDialog s={s} r={done} onClose={() => setDone(null)} />}
    </div>
  );
}

/** Forward the app's port over SSH and open it in the browser. */
async function openApp(s: SessionState, port: number) {
  const t = await forward(s, port);
  if (t) void openLocalPort(t.localPort);
}

function InstallAppDialog({ s, app, taken, onClose, onInstalled }: { s: SessionState; app: AppOption; taken: string[]; onClose: () => void; onInstalled: (r: AppInstalled) => void }) {
  const sid = s.info.id;
  const firstFree = (base: string) => { let n = base, i = 2; while (taken.includes(n)) n = `${base}-${i++}`; return n; };
  const [name, setName] = useState(firstFree(app.id));
  const [port, setPort] = useState(String(app.port));
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const clean = name.trim().toLowerCase().replace(/[^a-z0-9_-]+/g, "-").replace(/^-+|-+$/g, "");
  const portN = Number(port);
  const portOk = Number.isInteger(portN) && portN >= 1024 && portN <= 65535;
  const go = async () => {
    setBusy(true);
    setProblem(null);
    try {
      const r = await withSafetyOverride(
        (force) => runJob((p) => api.installApp(sid, app.id, clean, portN, force, p), {
          key: `app:${sid}:${clean}`, title: `Installing ${app.name}`, sid,
          done: (x) => (x.started ? `${app.name} is running` : { failed: `${app.name} was set up, but docker compose up failed` }),
        }),
        `${app.name} may not fit in this server's memory`,
      );
      if (r) onInstalled(r);
    } catch (e) {
      setProblem(asApiError(e).message.replace(/^Invalid input: /, ""));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Modal title={`Install ${app.name}`} subtitle={app.hint} keepOnOutsideClick onClose={onClose}
      footer={<><button className="btn" onClick={onClose}>Cancel</button><button className="btn primary" disabled={busy || !clean || taken.includes(clean) || !portOk} onClick={go}>{busy ? "Installing…" : "Install"}</button></>}>
      <div className="route-form">
        <label className="field">
          <span>Stack name</span>
          <input className="input mono" value={name} onChange={(e) => setName(e.target.value)} autoFocus />
          <span className="hint">{clean ? (taken.includes(clean) ? `${clean} already exists` : `/opt/stacks/${clean}`) : "Lowercase letters, digits, - and _"}</span>
        </label>
        <label className="field">
          <span>Port on the server</span>
          <input className="input mono" value={port} onChange={(e) => setPort(e.target.value.replace(/\D/g, ""))} inputMode="numeric" />
          <span className="hint">{portOk ? `Listens on 127.0.0.1:${portN} only; nothing is open to the internet.` : "1024 to 65535"}</span>
        </label>
        <ul className="app-steps">
          <li>Checks Docker Compose, the name, the port and free memory (~{app.memoryMb} MB).</li>
          <li>Writes <span className="mono">compose.yaml</span>, and generated passwords to <span className="mono">.env</span> (readable by you and root only). Orbital keeps no copy.</li>
          <li>Pulls <span className="mono">{app.images.join(", ")}</span> and starts it. The first pull can take a few minutes.</li>
        </ul>
        {problem && <div className="alert" style={{ whiteSpace: "pre-wrap" }}>{problem}</div>}
      </div>
    </Modal>
  );
}

function InstalledDialog({ s, r, onClose }: { s: SessionState; r: AppInstalled; onClose: () => void }) {
  const setNewWebsite = useStore((x) => x.setNewWebsite);
  const [copied, setCopied] = useState<string | null>(null);
  const hasNginx = !!s.tools?.some((t) => t.id === "nginx");
  const copy = (v: string, label: string) => { void navigator.clipboard?.writeText(v).catch(() => {}); setCopied(label); };
  return (
    <Modal title={r.started ? `${r.name} is running` : `${r.name} didn't start`} subtitle={r.started ? `On the server at 127.0.0.1:${r.port}.` : "The files are in place; Compose's output is below. Fix and Deploy it from Stacks."} keepOnOutsideClick onClose={onClose}
      footer={<>
        <button className="btn" onClick={onClose}>Close</button>
        {r.started && hasNginx && <button className="btn" onClick={() => { onClose(); setNewWebsite(s.info.id, r.port); }}><Globe size={13} /> Publish with a domain…</button>}
        {r.started && <button className="btn primary" onClick={() => void openApp(s, r.port)}><ExternalLink size={13} /> Open</button>}
      </>}>
      <div className="route-form">
        {r.shown.length > 0 && (
          <div className="app-secrets">
            <span className="faint">Save these now: Orbital doesn't keep them. They're also in <span className="mono">/opt/stacks/{r.name}/.env</span>.</span>
            {r.shown.map((x) => (
              <div key={x.label} className="app-secret">
                <span>{x.label}</span>
                <code className="mono">{x.value}</code>
                <button className="icon-btn" aria-label={`Copy ${x.label}`} onClick={() => copy(x.value, x.label)}>{copied === x.label ? <Check size={14} /> : <Copy size={14} />}</button>
              </div>
            ))}
          </div>
        )}
        {r.firstSteps && <p className="app-first">{r.firstSteps}</p>}
        {!r.started && <pre className="output stack-logs">{r.output}</pre>}
        {r.started && !hasNginx && <p className="faint">To reach it from a domain with HTTPS, install nginx from the + menu, then use New website with port {r.port}.</p>}
      </div>
    </Modal>
  );
}
