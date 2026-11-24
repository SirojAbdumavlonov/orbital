import { FileText, Layers, Play, Plus, RefreshCw, RotateCw, ScrollText, Square } from "lucide-react";
import { useEffect, useState } from "react";
import { confirm, Modal } from "../components/ui";
import { api, asApiError, Stack, StackAction } from "../lib/api";
import { runJob, useRunningJob } from "../lib/jobs";
import { SessionState, useStore } from "../store";

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
  const hasDocker = !!s.info.dockerVersion || !!s.tools?.some((t) => t.id === "docker");
  const load = () => api.stacks(sid).then((l) => { setList(l); setError(null); }, (e) => setError(asApiError(e).message));
  useEffect(() => { if (hasDocker) void load(); }, [sid, hasDocker]);

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
  return (
    <div className="view">
      <div className="toolbar view-toolbar">
        <span className="faint" style={{ fontSize: 12.5 }}>Compose projects on this server. New ones go to <span className="mono">/opt/stacks/&lt;name&gt;</span>.</span>
        <span className="spacer" />
        <button className="btn primary" onClick={() => setCreating(true)}><Plus size={13} /> New stack</button>
        <button className="icon-btn" aria-label="Refresh" onClick={() => void load()}><RefreshCw size={14} /></button>
      </div>
      <div className="scroll">
        {error && <div className="alert" style={{ margin: 16 }}>{error}</div>}
        {list && list.length === 0 && <div className="empty"><div><Layers size={28} /><h3>No Compose stacks yet</h3><p>Create one from a template, or deploy a compose file you already have on the server from its folder in Files.</p></div></div>}
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
