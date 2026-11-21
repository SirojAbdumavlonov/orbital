import { AlertTriangle, ArrowRight, Box, Copy, HardDrive, Network, Plug } from "lucide-react";
import { useEffect, useState } from "react";
import { JobProgress } from "../components/JobProgress";
import { Modal } from "../components/ui";
import { api, asApiError, Container, MoveMode, MovePreview } from "../lib/api";
import { runJob, useRunningJob } from "../lib/jobs";
import { SessionState, useStore } from "../store";

/**
 * Move or copy a container to another server with everything it needs: image,
 * volumes, bind-mounted files, network and options. Shows what will happen
 * (and what's in the way on the target) before anything starts; the move is a
 * job with live progress that keeps going if the dialog is closed.
 */
export function MoveContainerDialog({ s, c, onClose }: { s: SessionState; c: Container; onClose: () => void }) {
  const name = c.names.split(",")[0];
  const { sessions, profiles, connecting } = useStore();
  const key = `move:${s.info.id}:${c.id}`;
  const job = useRunningJob(key);
  const candidates = profiles.filter((p) => (p.protocol ?? "sftp") === "sftp" && p.id !== s.profile.id);
  const sessionOf = (pid: string) => Object.values(sessions).find((o) => o.profile.id === pid && !o.lost);
  const [pid, setPid] = useState(candidates.find((p) => sessionOf(p.id))?.id ?? candidates[0]?.id ?? "");
  const dst = pid ? sessionOf(pid) : undefined;
  const [mode, setMode] = useState<MoveMode>("move");
  const [preview, setPreview] = useState<MovePreview | null>(null);
  const [loading, setLoading] = useState(false);
  const [overwrite, setOverwrite] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    setPreview(null);
    setErr(null);
    if (!dst) return;
    let alive = true;
    setLoading(true);
    api.movePreview(s.info.id, c.id, dst.info.id).then(
      (p) => alive && setPreview(p),
      (e) => alive && setErr(asApiError(e).message),
    ).finally(() => alive && setLoading(false));
    return () => { alive = false; };
  }, [dst?.info.id, c.id]);

  async function connectTarget() {
    const here = s.info.id;
    await useStore.getState().connect(pid);
    useStore.setState({ active: here, globalView: null });
  }

  const cl = preview?.clashes;
  const blocked = !!cl && (cl.noDocker || cl.name || cl.ports.length > 0 || (cl.paths.length > 0 && !overwrite));
  const target = dst?.profile.name ?? "";

  async function go() {
    if (!dst || !preview) return;
    setErr(null);
    try {
      await runJob((p) => api.moveContainer(s.info.id, c.id, dst.info.id, mode, overwrite, p), {
        key,
        title: `${mode === "move" ? "Moving" : "Copying"} ${name} to ${target}`,
        sid: s.info.id,
        quietHere: false,
        done: () => `${name} runs on ${target}${mode === "move" ? `; it's stopped here (remove it when you're happy)` : ""}`,
      });
      onClose();
    } catch (e) {
      setErr(asApiError(e).message);
    }
  }

  const p = preview?.plan;
  return (
    <Modal
      title={`Move ${name} to another server`}
      subtitle={<>From <b>{s.profile.name}</b> · <span className="mono">{c.image}</span></>}
      wide
      keepOnOutsideClick
      onClose={onClose}
      footer={<>
        <button className="btn" onClick={onClose}>{job ? "Continue in the background" : "Cancel"}</button>
        <button className="btn primary" disabled={!preview || blocked || !!job} onClick={go}>{job ? "Working…" : mode === "move" ? `Move to ${target || "…"}` : `Copy to ${target || "…"}`}</button>
      </>}
    >
      <div className="move">
        <label className="field">
          <span>To</span>
          <div className="row">
            <select className="input" value={pid} onChange={(e) => setPid(e.target.value)} disabled={!!job}>
              {candidates.length === 0 && <option value="">No other SSH servers saved</option>}
              {candidates.map((p) => <option key={p.id} value={p.id}>{p.name}{sessionOf(p.id) ? "" : " (not connected)"}</option>)}
            </select>
            {pid && !dst && <button className="btn primary" disabled={!!connecting[pid]} onClick={connectTarget}><Plug size={13} /> {connecting[pid] ? "Connecting…" : "Connect"}</button>}
          </div>
        </label>

        <div className="segmented move-mode">
          <button className={mode === "move" ? "on" : ""} onClick={() => setMode("move")} disabled={!!job}><ArrowRight size={12} /> Move</button>
          <button className={mode === "copy" ? "on" : ""} onClick={() => setMode("copy")} disabled={!!job}><Copy size={12} /> Copy</button>
        </div>
        <p className="faint move-hint">
          {mode === "move"
            ? `Stops ${name} here first, so its data is copied as it is, then starts it on the target. It stays here, stopped; if anything fails it's started here again.`
            : `${name} keeps running here while its data is copied, so files changing right now may differ on the target. Good for a test copy.`}
        </p>

        {loading && <p className="faint">Reading {name} and checking {target}…</p>}
        {p && (
          <div className="move-plan">
            <div><Box size={14} /> Image <span className="mono">{p.image}</span> {p.repoDigest ? <span className="faint">pulled by exact version on the target</span> : <span className="faint">copied from this server</span>}</div>
            {p.copies.map((m) => (
              <div key={m.source}><HardDrive size={14} /> {m.volume ? <>Volume <span className="mono">{m.volume}</span></> : <span className="mono">{m.source}</span>} <span className="faint">→ {m.dest}</span></div>
            ))}
            {p.network && <div><Network size={14} /> Network <span className="mono">{p.network}</span></div>}
            <div className="faint">Same ports{p.hostPorts.length ? ` (${p.hostPorts.join(", ")})` : ""}, restart policy and options; {p.envCount} environment variables go through a private file (passwords in them aren't logged).</div>
            {p.notes.map((n) => <div key={n} className="faint">· {n}</div>)}
          </div>
        )}
        {cl && (cl.noDocker || cl.name || cl.ports.length > 0 || cl.paths.length > 0) && (
          <div className="alert">
            <AlertTriangle size={14} />
            <div>
              {cl.noDocker && <div>Docker isn't installed on {target}. Install it there first (the "+" menu → You may try → Docker).</div>}
              {cl.name && <div>{target} already has a container named <b>{name}</b>. Remove or rename it there first.</div>}
              {cl.ports.length > 0 && <div>Port {cl.ports.join(", ")} is already in use on {target}.</div>}
              {cl.paths.length > 0 && (
                <>
                  <div>These already exist on {target}: <span className="mono">{cl.paths.join(", ")}</span></div>
                  <label className="check"><input type="checkbox" checked={overwrite} onChange={(e) => setOverwrite(e.target.checked)} /> Overwrite them</label>
                </>
              )}
            </div>
          </div>
        )}
        {err && <div className="alert"><AlertTriangle size={14} /> <div>{err}</div></div>}
        {job && <JobProgress job={job} />}
      </div>
    </Modal>
  );
}
