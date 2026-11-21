import { AlertTriangle, Truck, Cable, SquareTerminal, ArrowDownWideNarrow, ArrowUpNarrowWide, Braces, Download, FileCode2, Pause, Pencil, Play, RefreshCw, RotateCw, ScrollText, Search, Square, Trash2, Upload } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { confirm, StateDot, DrawerClose } from "../components/ui";
import { Modal } from "../components/ui";
import { api, asApiError, ComposeAction, ComposeTarget, Container, ContainerAction, ContainerStats, ExecOutput } from "../lib/api";
import { crashFacts, explain } from "../lib/containerCrash";
import { publishedPorts } from "../lib/ports";
import { forward } from "./TunnelsView";
import { runJob, useRunningJob } from "../lib/jobs";
import { JobProgress } from "../components/JobProgress";
import { VSplit } from "../components/VSplit";
import { rangeIso, useLogPane } from "../components/LogPane";
import { MoveContainerDialog } from "./MoveContainer";
import { SessionState, useStore } from "../store";
import { useKept } from "../lib/kept";

type StateFilter = "all" | "running" | "stopped" | "paused";
type SortKey = "name" | "state" | "image" | "cpu" | "memory";

/** Running first when sorting by state ascending. */
const STATE_RANK: Record<string, number> = { running: 0, restarting: 1, paused: 2, created: 3, exited: 4, dead: 5 };
const isStopped = (c: Container) => ["exited", "created", "dead"].includes(c.state);

/** Docker Compose records where a container came from in its labels. */
function composeOf(inspect: any): ComposeTarget | null {
  const l = inspect?.Config?.Labels ?? {};
  const project = l["com.docker.compose.project"];
  if (!project) return null;
  const files = String(l["com.docker.compose.project.config_files"] ?? "").split(",").map((f: string) => f.trim()).filter(Boolean);
  return { project, files, workingDir: l["com.docker.compose.project.working_dir"] ?? null, service: l["com.docker.compose.service"] ?? null };
}

function ComposePane({ s, inspect }: { s: SessionState; inspect: any }) {
  const { openFile } = useStore();
  const sid = s.info.id;
  const job = useRunningJob(`compose:${sid}`);
  const target = composeOf(inspect);
  const file = target?.files[0] ?? null;
  const [text, setText] = useState<string | null>(null);
  const [busy, setBusy] = useState<ComposeAction | null>(null);
  const [out, setOut] = useState<{ title: string; o: ExecOutput } | null>(null);
  useEffect(() => {
    setText(null);
    if (file) api.readFile(sid, file).then((f) => setText(f.content)).catch((e) => setText(`# Couldn't read ${file}: ${asApiError(e).message}`));
  }, [sid, file]);

  if (!inspect) return <pre className="logs">Loading…</pre>;
  if (!target) return <div className="empty"><p>This container wasn't started with Docker Compose.</p></div>;

  async function run(action: ComposeAction, scope: "project" | "service") {
    if (!target) return;
    const t = scope === "project" ? { ...target, service: null } : target;
    if (action === "up") {
      const ok = await confirm({
        title: `Apply ${t.project}?`,
        body: "docker compose up -d recreates every container whose configuration or image changed. They are briefly unavailable while that happens.",
        confirmLabel: "Apply",
      });
      if (!ok) return;
    }
    setBusy(action);
    try {
      const what = `${action === "up" ? "Apply" : action === "restart" ? "Restart" : "Pull"} ${t.service ?? t.project}`;
      const o = await runJob((p) => api.composeAction(sid, t, action, p), {
        key: `compose:${sid}`,
        title: what,
        sid,
        done: (r) => (r.exitCode === 0 ? `${what}: done` : { failed: "Compose reported an error" }),
      });
      setOut({ title: o.exitCode === 0 ? `${what}: done` : `${what}: failed`, o });
    } catch {
      // The job reports the error.
    } finally {
      setBusy(null);
    }
  }

  return (
    <>
      <dl className="kv">
        <dt>Project</dt><dd className="mono">{target.project}</dd>
        <dt>Service</dt><dd className="mono">{target.service ?? "—"}</dd>
        <dt>Folder</dt><dd className="mono">{target.workingDir ?? "—"}</dd>
        <dt>Files</dt><dd className="mono">{target.files.map((f) => <div key={f}><a className="link" onClick={() => openFile(sid, f)}>{f}</a></div>)}</dd>
      </dl>
      <div className="actions">
        {file && <button className="btn primary" onClick={() => openFile(sid, file)} title="Open in the editor; saving checks it with docker compose config"><Pencil size={13} /> Edit</button>}
        <button className="btn" disabled={!!busy} onClick={() => run("up", "project")} title="docker compose up -d: create or recreate what changed"><Upload size={13} className={busy === "up" ? "spin" : ""} /> Apply changes</button>
        {target.service && <button className="btn" disabled={!!busy} onClick={() => run("restart", "service")}><RotateCw size={13} className={busy === "restart" ? "spin" : ""} /> Restart {target.service}</button>}
        <button className="btn" disabled={!!busy} onClick={() => run("pull", "project")} title="Download newer images; apply to use them"><Download size={13} className={busy === "pull" ? "spin" : ""} /> Pull images</button>
      </div>
      {job && <JobProgress job={job} compact />}
      <pre className="logs">{file ? text ?? "Loading…" : "Docker didn't record which file this project came from."}</pre>
      {out && (
        <Modal title={out.title} wide onClose={() => setOut(null)} footer={<button className="btn primary" onClick={() => setOut(null)}>Close</button>}>
          <pre className="output" style={{ maxHeight: 420 }}>{[out.o.stdout, out.o.stderr].filter((x) => x.trim()).join("\n") || "docker compose printed nothing."}</pre>
        </Modal>
      )}
    </>
  );
}

export function DockerView({ s }: { s: SessionState }) {
  const sid = s.info.id;
  const job = useRunningJob(`container:${sid}`);
  const [list, setList] = useState<Container[]>([]);
  const [stats, setStats] = useState<ContainerStats[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [q, setQ] = useKept(`${s.info.id}:docker:q`, "");
  const [sel, setSel] = useKept<string | null>(`${s.info.id}:docker:sel`, null);
  const [pane, setPane] = useKept<"inspect" | "compose">(`${s.info.id}:docker:pane`, "inspect");
  const [stateFilter, setStateFilter] = useKept<StateFilter>(`${s.info.id}:docker:stateFilter`, "all");
  const [sortKey, setSortKey] = useKept<SortKey>(`${s.info.id}:docker:sortKey`, "name");
  const [desc, setDesc] = useKept(`${s.info.id}:docker:desc`, false);
  const [inspect, setInspect] = useState<any>(null);
  const [busy, setBusy] = useState(false);
  // Kept too, so the Move dialog survives connecting the target (which switches servers for a moment).
  const [movingId, setMovingId] = useKept<string | null>(`${s.info.id}:docker:moving`, null);

  const load = async () => {
    if (!s.info.dockerVersion) return;
    setLoading(true);
    try {
      setList(await api.listContainers(sid));
      setError(null);
      api.containerStats(sid).then(setStats).catch(() => {});
    } catch (e) {
      setError(asApiError(e).message);
    } finally {
      setLoading(false);
    }
  };
  useEffect(() => { load(); const t = setInterval(load, 15000); return () => clearInterval(t); }, [sid]);
  const cur = list.find((c) => c.id === sel);
  const moving = list.find((c) => c.id === movingId) ?? null;
  useEffect(() => {
    if (!cur) return;
    if (!cur.composeProject) setPane("inspect");
    setInspect(null);
    api.inspectContainer(sid, cur.id).then(setInspect).catch(() => {});
    // `cur` too: a kept selection only resolves once the container list has loaded.
  }, [sel, !!cur]);

  const log = useLogPane({
    sid,
    id: cur?.id ?? null,
    name: cur?.names.split(",")[0] ?? "container",
    follow: cur ? { type: "container", id: cur.id } : null,
    load: (r) => (r ? api.containerLogs(sid, cur!.id, 5000, rangeIso(r.from), rangeIso(r.to)) : api.containerLogs(sid, cur!.id, 400)),
  });
  const logs = log.text;

  const statOf = (c: Container) => stats.find((x) => c.id.startsWith(x.id) || x.name === c.names);
  const groups = useMemo(() => {
    const num = (v?: string) => parseFloat(v ?? "") || 0;
    const key = (c: Container): string | number => {
      switch (sortKey) {
        case "state": return STATE_RANK[c.state] ?? 9;
        case "image": return c.image;
        case "cpu": return num(statOf(c)?.cpuPercent);
        case "memory": return num(statOf(c)?.memPercent);
        default: return c.names;
      }
    };
    const cmp = (a: Container, b: Container) => {
      const x = key(a), y = key(b);
      const r = typeof x === "number" && typeof y === "number" ? x - y : String(x).localeCompare(String(y));
      return (desc ? -r : r) || a.names.localeCompare(b.names);
    };
    const shown = list.filter((c) => {
      if (stateFilter === "running" && c.state !== "running") return false;
      if (stateFilter === "stopped" && !isStopped(c)) return false;
      if (stateFilter === "paused" && c.state !== "paused") return false;
      return !q || `${c.names} ${c.image}`.toLowerCase().includes(q.toLowerCase());
    });
    const g = new Map<string, Container[]>();
    for (const c of shown) {
      const k = c.composeProject ?? "";
      g.set(k, [...(g.get(k) ?? []), c]);
    }
    return [...g.entries()]
      .map(([k, cs]) => [k, cs.sort(cmp)] as const)
      .sort(([a], [b]) => (a === "" ? 1 : b === "" ? -1 : a.localeCompare(b)));
  }, [list, stats, q, stateFilter, sortKey, desc]);
  const counts = { running: list.filter((c) => c.state === "running").length, stopped: list.filter(isStopped).length, paused: list.filter((c) => c.state === "paused").length };

  async function act(action: ContainerAction) {
    if (!cur) return;
    const destructive = action === "remove" || action === "forceRemove" || action === "stop";
    if (destructive) {
      const ok = await confirm({
        title: action === "stop" ? `Stop ${cur.names}?` : `Remove ${cur.names}?`,
        body: action === "stop"
          ? "The container stops serving until you start it again."
          : <>The container is deleted. Data in named volumes is kept; anything written only inside the container is lost.{action === "forceRemove" && " It will be killed first if running."}</>,
        confirmLabel: action === "stop" ? "Stop container" : "Remove container",
        danger: true,
        requireText: action === "stop" ? undefined : cur.names,
      });
      if (!ok) return;
    }
    setBusy(true);
    try {
      await runJob((p) => api.containerAction(sid, cur.id, action, p), {
        key: `container:${sid}`,
        title: `${cur.names}: ${action === "forceRemove" ? "remove" : action}`,
        sid,
        quietHere: false,
        done: () => `${cur.names}: ${action === "forceRemove" ? "removed" : action} done`,
      });
      if (action === "remove" || action === "forceRemove") setSel(null);
    } catch {
      // The job reports the error.
    } finally {
      setBusy(false);
      load();
    }
  }

  if (!s.info.dockerVersion) {
    return <div className="empty"><div><h3>Docker isn't available on this server</h3><p>Orbital looks for the <span className="mono">docker</span> command. Install Docker Engine, or add {s.profile.username} to the <span className="mono">docker</span> group, then reconnect.</p></div></div>;
  }

  return (
    <div className="view">
      <div className="toolbar">
        <div className="search"><Search size={14} /><input className="input" placeholder="Find a container or image" value={q} onChange={(e) => setQ(e.target.value)} /></div>
        <div className="segmented" aria-label="Show containers">
          {([["all", `All ${list.length}`], ["running", `Running ${counts.running}`], ["stopped", `Stopped ${counts.stopped}`], ["paused", `Paused ${counts.paused}`]] as [StateFilter, string][]).map(([id, label]) => (
            <button key={id} className={stateFilter === id ? "on" : ""} onClick={() => setStateFilter(id)}>{label}</button>
          ))}
        </div>
        <span className="spacer" />
        <label className="faint sort">Sort by
          <select className="input" value={sortKey} onChange={(e) => setSortKey(e.target.value as SortKey)}>
            <option value="name">Name</option><option value="state">State</option><option value="image">Image</option><option value="cpu">CPU</option><option value="memory">Memory</option>
          </select>
        </label>
        <button className="icon-btn" aria-label={desc ? "Descending; switch to ascending" : "Ascending; switch to descending"} title={desc ? "Descending" : "Ascending"} onClick={() => setDesc(!desc)}>
          {desc ? <ArrowDownWideNarrow size={15} /> : <ArrowUpNarrowWide size={15} />}
        </button>
        <button className="icon-btn" aria-label="Refresh" onClick={load}><RefreshCw size={14} className={loading ? "spin" : ""} /></button>
      </div>
      {error && <div className="alert" style={{ margin: 12 }}>{error}</div>}
      <div className="split">
        <div className="scroll">
          <table className="table">
            <thead><tr><th>Container</th><th>State</th><th>Image</th><th className="num">CPU</th><th className="num">Memory</th><th>Ports</th></tr></thead>
            {groups.map(([proj, cs]) => (
              <tbody key={proj || "_"}>
                {proj && <tr><td colSpan={6} className="faint" style={{ paddingTop: 14, fontSize: 12, fontWeight: 500 }}>Compose project {proj}</td></tr>}
                {cs.map((c) => {
                  const st = statOf(c);
                  return (
                    <tr key={c.id} className={`click ${sel === c.id ? "sel" : ""}`} onClick={() => setSel(c.id)}>
                      <td className="mono nowrap">{c.names}</td>
                      <td className="nowrap"><StateDot state={c.state} label={c.status} nonzero={c.state === "exited" && !c.status.includes("(0)")} /></td>
                      <td className="muted mono" style={{ maxWidth: 220, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{c.image}</td>
                      <td className="num">{st?.cpuPercent ?? "—"}</td>
                      <td className="num">{st ? st.memUsage.split(" / ")[0] : "—"}</td>
                      <td className="faint mono" style={{ fontSize: 11.5, maxWidth: 200, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{c.ports || "—"}</td>
                    </tr>
                  );
                })}
              </tbody>
            ))}
          </table>
          {!loading && list.length === 0 && !error && <div className="empty"><p>No containers on this server yet.</p></div>}
          {list.length > 0 && groups.length === 0 && <div className="empty" style={{ flex: "none" }}><p>No containers match.</p></div>}
          <p className="footnote">Docker {s.info.dockerVersion} · {counts.running} of {list.length} running</p>
        </div>
        <aside className="drawer">
          {!cur ? (
            <div className="empty"><p>Select a container to see its logs and controls.</p></div>
          ) : (
            <>
              <div className="drawer-head">
                <DrawerClose onClose={() => setSel(null)} />
                <h3 className="mono">{cur.names}</h3>
                <div className="muted mono" style={{ fontSize: 12 }}>{cur.id.slice(0, 12)} · {cur.image}</div>
              </div>
              <div className="actions">
                {cur.state !== "running" && cur.state !== "paused" && <button className="btn primary" disabled={busy} onClick={() => act("start")}><Play size={13} /> Start</button>}
                {cur.state === "running" && <button className="btn" disabled={busy} onClick={() => act("restart")}><RotateCw size={13} /> Restart</button>}
                {cur.state === "running" && <button className="btn" disabled={busy} onClick={() => act("pause")}><Pause size={13} /> Pause</button>}
                {cur.state === "paused" && <button className="btn" disabled={busy} onClick={() => act("unpause")}><Play size={13} /> Resume</button>}
                {cur.state === "running" && <button className="btn danger" disabled={busy} onClick={() => act("stop")}><Square size={13} /> Stop</button>}
                <button className="btn danger" disabled={busy} onClick={() => act(cur.state === "running" ? "forceRemove" : "remove")}><Trash2 size={13} /> Remove</button>
                {cur.state === "running" && (
                  <button className="btn" title="Open a shell inside this container (docker exec)" onClick={() => useStore.getState().openTerminal(sid, null, s.info.elevation, { kind: "container", id: cur.names.split(",")[0] })}>
                    <SquareTerminal size={13} /> Shell
                  </button>
                )}
                <button className="btn" title="Recreate it on another server: image, volumes, files, network and options" onClick={() => setMovingId(cur.id)}><Truck size={13} /> Move to server…</button>
              </div>
              {cur.state === "running" && publishedPorts(cur.ports).length > 0 && (
                <div className="port-forward">
                  <span className="faint">Use on this computer:</span>
                  {publishedPorts(cur.ports).map((p) => (
                    <button key={`${p.host}:${p.port}`} className="chip" title={`Forward port ${p.port} on the server to localhost on this computer, through SSH${p.localOnly ? " (it only listens on the server itself)" : ""}`} onClick={() => void forward(s, p.port)}>
                      <Cable size={12} /> {p.port}{p.localOnly ? " · server-only" : ""}
                    </button>
                  ))}
                </div>
              )}
              {job && <JobProgress job={job} compact />}
              <CrashPanel inspect={inspect} logs={logs} />
              <VSplit
                id="docker"
                top={{
                  title: <span className="vsplit-title"><ScrollText size={13} /> Logs</span>,
                  extra: log.extra,
                  body: log.body,
                }}
                bottom={{
                  title: (
                    <div className="segmented">
                      <button className={pane === "inspect" ? "on" : ""} onClick={() => setPane("inspect")}><Braces size={12} /> Inspect</button>
                      {cur.composeProject && <button className={pane === "compose" ? "on" : ""} onClick={() => setPane("compose")}><FileCode2 size={12} /> Compose</button>}
                    </div>
                  ),
                  body: pane === "compose" && cur.composeProject
                    ? <div className="vsplit-body"><ComposePane s={s} inspect={inspect} /></div>
                    : <pre className="logs">{inspect ? JSON.stringify(inspect, null, 2) : "Loading…"}</pre>,
                }}
              />
            </>
          )}
        </aside>
      </div>
      {moving && <MoveContainerDialog s={s} c={moving} onClose={() => setMovingId(null)} />}
    </div>
  );
}


/** "Why did it stop?": shown for a container that crashed, restarts or is unhealthy. */
function CrashPanel({ inspect, logs }: { inspect: any; logs: string }) {
  const facts = crashFacts(inspect);
  const e = facts && explain(facts, logs);
  if (!facts || !e) return null;
  return (
    <div className={`crash ${e.level}`}>
      <div className="crash-head"><AlertTriangle size={15} /> <b>{e.headline}</b>{facts.finishedAt && <span className="faint"> · {new Date(facts.finishedAt).toLocaleString([], { dateStyle: "medium", timeStyle: "short" })}</span>}</div>
      <ul>{e.hints.map((h, i) => <li key={i}>{h}</li>)}</ul>
    </div>
  );
}

