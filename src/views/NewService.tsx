import { Plus, Trash2 } from "lucide-react";
import { useState } from "react";
import { Modal } from "../components/ui";
import { api, asApiError, RestartPolicy, ServiceSpec } from "../lib/api";
import { runJob } from "../lib/jobs";
import { SessionState } from "../store";

// Services → New service: turn a command into a systemd service that starts
// at boot and restarts when it stops (orbital_core::units).

/** A name for the unit from what the user typed ("Shop API" → "shop-api"). */
export function unitName(s: string): string {
  return s.trim().toLowerCase().replace(/\.service$/, "").replace(/[^a-z0-9_.@-]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 63);
}

/** What the unit file will look like (the engine writes the real one; secrets aren't shown). */
export function previewUnit(spec: ServiceSpec): string {
  const q = (x: string) => `"${x.replace(/\\/g, "\\\\").replace(/"/g, '\\"').replace(/%/g, "%%").replace(/\$/g, "$$$$")}"`;
  const lines = ["[Unit]", `Description=${spec.description || spec.name}`, "After=network-online.target", "", "[Service]"];
  if (spec.workdir) lines.push(`WorkingDirectory=${spec.workdir.replace(/%/g, "%%")}`);
  if (spec.user && spec.user !== "root") lines.push(`User=${spec.user}`);
  if (spec.env.some(([k]) => k.trim())) lines.push(`EnvironmentFile=/etc/orbital/env/${spec.name}.env   # root only`);
  lines.push(`ExecStart=/bin/sh -c ${q(spec.command)}`);
  lines.push(spec.restart === "no" ? "Restart=no" : `Restart=${spec.restart === "always" ? "always" : "on-failure"}`);
  lines.push("", "[Install]", "WantedBy=multi-user.target");
  return lines.join("\n");
}

export function NewServiceDialog({ s, onClose, onCreated }: { s: SessionState; onClose: () => void; onCreated: (name: string) => void }) {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [command, setCommand] = useState("");
  const [workdir, setWorkdir] = useState("");
  const [user, setUser] = useState(s.profile.username === "root" ? "" : s.profile.username);
  const [restart, setRestart] = useState<RestartPolicy>("always");
  const [env, setEnv] = useState<[string, string][]>([["", ""]]);
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const unit = unitName(name);
  const spec: ServiceSpec = { name: unit, description: description.trim(), command: command.trim(), workdir: workdir.trim() || null, user: user.trim() || null, restart, env: env.filter(([k]) => k.trim()) };
  const create = async () => {
    setBusy(true);
    setProblem(null);
    try {
      const [running, out] = await runJob((p) => api.createService(s.info.id, spec, p), {
        key: `new-service:${s.info.id}`, title: `Creating ${unit}.service`, sid: s.info.id,
        done: ([ok]) => (ok ? `${unit}.service is running and starts at boot` : { failed: `${unit}.service was created but isn't running; see its logs` }),
      });
      if (!running) setProblem(`Created, but it isn't running:\n${out.trim()}`);
      onCreated(`${unit}.service`);
    } catch (e) {
      const err = asApiError(e);
      setProblem(err.kind === "ValidationFailed" ? `systemd rejected it, so nothing was kept:\n${err.data?.output ?? err.message}` : err.message.replace(/^Invalid input: /, ""));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Modal
      title="New service"
      subtitle="Run a command as a service: started now and at every boot, and restarted if it stops."
      wide
      keepOnOutsideClick
      onClose={onClose}
      footer={<><button className="btn" onClick={onClose}>Cancel</button><button className="btn primary" disabled={busy || !unit || !command.trim()} onClick={create}>{busy ? "Creating…" : "Create and start"}</button></>}
    >
      <div className="route-form">
        <div className="grid2">
          <label className="field"><span>Name</span><input className="input mono" value={name} onChange={(e) => setName(e.target.value)} placeholder="shop-api" autoFocus />{unit && <span className="hint mono">{unit}.service</span>}</label>
          <label className="field"><span>Description (optional)</span><input className="input" value={description} onChange={(e) => setDescription(e.target.value)} placeholder="Shop API" /></label>
        </div>
        <label className="field"><span>Command</span><input className="input mono" value={command} onChange={(e) => setCommand(e.target.value)} placeholder="node server.js --port 3000" spellCheck={false} /><span className="hint">Runs with /bin/sh, so pipes, && and $VARIABLES work. It should keep running (not start something in the background and exit).</span></label>
        <div className="grid2">
          <label className="field"><span>Folder (optional)</span><input className="input mono" value={workdir} onChange={(e) => setWorkdir(e.target.value)} placeholder="/srv/shop" /></label>
          <label className="field"><span>Run as</span><input className="input mono" value={user} onChange={(e) => setUser(e.target.value)} placeholder="root" /><span className="hint">A user on the server; empty for root.</span></label>
        </div>
        <div className="field">
          <span>When it stops</span>
          <div className="segmented" style={{ alignSelf: "flex-start" }}>
            {([["always", "Always restart"], ["onFailure", "Restart if it fails"], ["no", "Leave it stopped"]] as [RestartPolicy, string][]).map(([k, label]) => (
              <button key={k} className={restart === k ? "on" : ""} onClick={() => setRestart(k)}>{label}</button>
            ))}
          </div>
        </div>
        <div className="field">
          <span>Environment variables (optional)</span>
          {env.map(([k, v], i) => (
            <div key={i} className="env-row">
              <input className="input mono" value={k} placeholder="NAME" onChange={(e) => setEnv(env.map((x, j) => (j === i ? [e.target.value, x[1]] : x)))} />
              <input className="input mono" value={v} placeholder="value" onChange={(e) => setEnv(env.map((x, j) => (j === i ? [x[0], e.target.value] : x)))} />
              <button className="icon-btn" aria-label="Remove variable" onClick={() => setEnv(env.length > 1 ? env.filter((_, j) => j !== i) : [["", ""]])}><Trash2 size={13} /></button>
            </div>
          ))}
          <button className="btn ghost" style={{ alignSelf: "flex-start" }} onClick={() => setEnv([...env, ["", ""]])}><Plus size={13} /> Add a variable</button>
          <span className="hint">Kept in /etc/orbital/env/{unit || "name"}.env, readable by root only: fine for passwords and tokens.</span>
        </div>
        {unit && command.trim() && (
          <div className="field"><span>The unit file</span><pre className="route-diff mono">{previewUnit(spec)}</pre></div>
        )}
        {problem && <div className="alert" style={{ whiteSpace: "pre-wrap" }}>{problem}</div>}
      </div>
    </Modal>
  );
}
