import { AlertTriangle, ArrowRight, Box, Copy, HardDrive, Info, Network, Plug, Plus, RotateCcw, SlidersHorizontal, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { JobProgress } from "../components/JobProgress";
import { Modal } from "../components/ui";
import { api, asApiError, Container, MoveMode, MoveOptions, MovePreview, PortMap } from "../lib/api";
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
  // The dialog's adjustments (null = as the container has it).
  const [ports, setPorts] = useState<PortMap[] | null>(null);
  const [args, setArgs] = useState<string[] | null>(null);
  const [envSet, setEnvSet] = useState<Record<string, string>>({});
  const [envRemove, setEnvRemove] = useState<string[]>([]);
  const [newVar, setNewVar] = useState({ key: "", value: "" });
  const options: MoveOptions = { ...(ports ? { ports } : {}), ...(args ? { args } : {}), envSet, envRemove };
  const optionsKey = JSON.stringify(options);
  const first = useRef(true);

  useEffect(() => {
    setErr(null);
    if (!dst) return setPreview(null);
    let alive = true;
    // Re-check the target shortly after each change (ports in use, …).
    const t = setTimeout(() => {
      setLoading(true);
      api.movePreview(s.info.id, c.id, dst.info.id, options).then(
        (p) => alive && setPreview(p),
        (e) => alive && setErr(asApiError(e).message),
      ).finally(() => alive && setLoading(false));
    }, first.current ? 0 : 500);
    first.current = false;
    return () => { alive = false; clearTimeout(t); };
  }, [dst?.info.id, c.id, optionsKey]);

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
      await runJob((p) => api.moveContainer(s.info.id, c.id, dst.info.id, mode, overwrite, options, p), {
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
            <div className="faint">Same restart policy, volumes and options; {p.envCount} environment variables go through a private file (passwords in them aren't logged).</div>
            {p.notes.map((n) => <div key={n} className="faint">· {n}</div>)}
          </div>
        )}
        {p && p.hints.length > 0 && (
          <div className="move-hints">
            {p.hints.map((h) => <div key={h}><Info size={14} /> <span>{h}</span></div>)}
          </div>
        )}
        {p && (
          <details className="move-adjust" open={p.hints.length > 0 || ports !== null || args !== null || envRemove.length > 0 || Object.keys(envSet).length > 0}>
            <summary><SlidersHorizontal size={13} /> Adjust for the new server: ports, arguments, environment</summary>

            <div className="adjust-title">Ports</div>
            {(ports ?? p.ports).length === 0 && <p className="faint">It publishes no ports.</p>}
            {(ports ?? p.ports).map((pm, i) => {
              const list = ports ?? p.ports;
              const set = (n: Partial<PortMap>) => setPorts(list.map((x, j) => (j === i ? { ...x, ...n } : x)));
              return (
                <div className="adjust-row" key={i}>
                  <span className="mono adjust-cport">{pm.container}</span>
                  <span className="faint">on</span>
                  <select className="input" value={pm.hostIp === "127.0.0.1" ? "local" : pm.hostIp === "" ? "public" : "other"} onChange={(e) => set({ hostIp: e.target.value === "local" ? "127.0.0.1" : "" })}>
                    <option value="public">every address (public)</option>
                    <option value="local">this server only (127.0.0.1)</option>
                    {pm.hostIp !== "" && pm.hostIp !== "127.0.0.1" && <option value="other">{pm.hostIp}</option>}
                  </select>
                  <span className="faint">port</span>
                  <input className="input mono adjust-port" inputMode="numeric" value={pm.hostPort ?? ""} placeholder="any" onChange={(e) => set({ hostPort: e.target.value ? Number(e.target.value.replace(/\D/g, "").slice(0, 5)) || null : null })} />
                  <button className="icon-btn" aria-label="Don't publish this port" title="Don't publish this port" onClick={() => setPorts(list.filter((_, j) => j !== i))}><X size={13} /></button>
                </div>
              );
            })}
            {(ports ?? p.ports).some((x) => x.hostIp === "") && <p className="faint adjust-note">Public ports also need a rule in the new server's cloud firewall (for example Oracle Cloud's security list) to be reachable from outside.</p>}

            <div className="adjust-title">Arguments <span className="faint">after the image</span></div>
            {(args ?? p.args).map((a, i) => {
              const list = args ?? p.args;
              return (
                <div className="adjust-row" key={i}>
                  <input className="input mono" value={a} onChange={(e) => setArgs(list.map((x, j) => (j === i ? e.target.value : x)))} />
                  <button className="icon-btn" aria-label="Remove this argument" title="Remove" onClick={() => setArgs(list.filter((_, j) => j !== i))}><X size={13} /></button>
                </div>
              );
            })}
            <button className="btn ghost adjust-add" onClick={() => setArgs([...(args ?? p.args), ""])}><Plus size={12} /> Add an argument</button>

            <div className="adjust-title">Environment</div>
            {p.envVars.map((v) => {
              const removed = envRemove.includes(v.key);
              return (
                <div className={`adjust-row ${removed ? "removed" : ""}`} key={v.key}>
                  <span className="mono adjust-key">{v.key}</span>
                  <input
                    className="input mono"
                    disabled={removed}
                    value={envSet[v.key] ?? v.value ?? ""}
                    placeholder={v.secret ? "hidden; type to replace it" : ""}
                    onChange={(e) => setEnvSet({ ...envSet, [v.key]: e.target.value })}
                  />
                  {removed
                    ? <button className="icon-btn" aria-label="Keep it" title="Keep it" onClick={() => setEnvRemove(envRemove.filter((k) => k !== v.key))}><RotateCcw size={13} /></button>
                    : <button className="icon-btn" aria-label="Don't pass this variable" title="Don't pass it" onClick={() => { setEnvRemove([...envRemove, v.key]); const { [v.key]: _, ...rest } = envSet; setEnvSet(rest); }}><X size={13} /></button>}
                </div>
              );
            })}
            <div className="adjust-row">
              <input className="input mono adjust-key" placeholder="NEW_VARIABLE" value={newVar.key} onChange={(e) => setNewVar({ ...newVar, key: e.target.value.replace(/[^A-Za-z0-9_]/g, "") })} />
              <input className="input mono" placeholder="value" value={newVar.value} onChange={(e) => setNewVar({ ...newVar, value: e.target.value })} />
              <button className="btn" disabled={!/^[A-Za-z_]/.test(newVar.key)} onClick={() => { setEnvSet({ ...envSet, [newVar.key]: newVar.value }); setEnvRemove(envRemove.filter((k) => k !== newVar.key)); setNewVar({ key: "", value: "" }); }}><Plus size={12} /> Add</button>
            </div>
            {(ports || args || envRemove.length > 0 || Object.keys(envSet).length > 0) && (
              <button className="btn ghost adjust-add" onClick={() => { setPorts(null); setArgs(null); setEnvSet({}); setEnvRemove([]); }}><RotateCcw size={12} /> Back to the container's own settings</button>
            )}
          </details>
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
