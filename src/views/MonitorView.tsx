import { HistoryChart } from "../components/HistoryChart";
import { RangePicker } from "../components/RangePicker";
import { diskForecast, useRangeHistory } from "../lib/metricHistory";
import { AlertTriangle, ArrowUp, CheckCircle2, File, Folder, FolderOpen, RefreshCw } from "lucide-react";
import { useEffect, useState } from "react";
import { Bar, confirm, LiveNumber, Modal, Sparkline } from "../components/ui";
import { runJob } from "../lib/jobs";
import { api, DiskSeries, DiskUsage, Metrics, ProcessReport } from "../lib/api";
import { asApiError } from "../lib/api";
import { useRemote } from "../lib/remote";
import { bytes, duration, rate } from "../lib/format";
import { isSsh, SessionState, useStore } from "../store";
import { useKept } from "../lib/kept";

const VIRT: Record<string, string> = { none: "Physical machine", kvm: "Virtual machine (KVM)", qemu: "Virtual machine (QEMU)", xen: "Virtual machine (Xen)", vmware: "Virtual machine (VMware)", microsoft: "Virtual machine (Hyper-V)", oracle: "Virtual machine (VirtualBox)", amazon: "Virtual machine (AWS Nitro)", lxc: "Container (LXC)", docker: "Container (Docker)", openvz: "Container (OpenVZ)", wsl: "WSL" };

type UsageView = "byProgram" | "byService" | "byUser" | "processes";

/** Ask, then stop a process: SIGTERM, and SIGKILL if the user asks to force it. */
async function stopProcess(s: SessionState, pid: number, name: string) {
  const ok = await confirm({
    title: `Stop ${name} (PID ${pid})?`,
    body: "It is asked to quit (SIGTERM), like Ctrl+C. If it belongs to a service, systemd may start it again; stop the service from Services instead.",
    confirmLabel: "Stop process",
    danger: true,
  });
  if (!ok) return;
  const { toast } = useStore.getState();
  try {
    await api.stopProcess(s.info.id, pid, false);
    toast({ kind: "success", title: `Asked ${name} to stop`, body: "If it doesn't, stop it again; Orbital then offers to force it." });
  } catch (e) {
    const err = asApiError(e);
    if (err.kind === "Remote") {
      const force = await confirm({ title: `${name} didn't stop. Force it?`, body: "SIGKILL ends it at once, without letting it save anything.", confirmLabel: "Force stop", danger: true });
      if (force) await api.stopProcess(s.info.id, pid, true).then(() => toast({ kind: "success", title: `${name} was forced to stop` }), (x) => toast({ kind: "error", title: "Couldn't stop it", body: asApiError(x).message }));
    } else toast({ kind: "error", title: `Couldn't stop ${name}`, body: err.message });
  }
}

/** What is using the CPU and memory right now, by program, service, user or process. */
function UsageByProgram({ s, memTotal }: { s: SessionState; memTotal: number }) {
  const [r, setR] = useState<ProcessReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [view, setView] = useKept<UsageView>(`${s.info.id}:usage:view`, "byProgram");
  const [sort, setSort] = useState<"cpu" | "memory">("cpu");
  useEffect(() => {
    let stop = false;
    let timer: number;
    const tick = async () => {
      try {
        const next = await api.processUsage(s.info.id);
        if (!stop) { setR(next); setError(null); }
      } catch (e) {
        if (!stop) setError(asApiError(e).message);
      }
      if (!stop) timer = window.setTimeout(tick, 5000);
    };
    tick();
    return () => { stop = true; clearTimeout(timer); };
  }, [s.info.id]);

  const rows = !r ? [] : view === "processes"
    ? r.processes.map((p) => ({ key: String(p.pid), pid: p.pid as number | null, name: p.command, detail: `PID ${p.pid} · ${p.user}${p.unit ? ` · ${p.unit}` : ""}`, cpu: p.cpu, rss: p.rssBytes, count: 1 }))
    : r[view].map((g) => ({ key: g.name, pid: null as number | null, name: g.name, detail: `${g.processes} process${g.processes === 1 ? "" : "es"}`, cpu: g.cpu, rss: g.rssBytes, count: g.processes }));
  const sorted = [...rows].sort((a, b) => (sort === "cpu" ? b.cpu - a.cpu || b.rss - a.rss : b.rss - a.rss)).slice(0, 15);
  const capacity = (r?.cores ?? 1) * 100;
  return (
    <section className="panel span-12">
      <h4>What uses the CPU and memory
        <span className="faint">{r ? (r.method === "sampled" ? "measured over the last second" : "as reported by ps") : ""}</span>
        <span style={{ flex: 1 }} />
        <div className="segmented">
          {([["byProgram", "Programs"], ["byService", "Services"], ["byUser", "Users"], ["processes", "Processes"]] as [UsageView, string][]).map(([id, label]) => (
            <button key={id} className={view === id ? "on" : ""} onClick={() => setView(id)}>{label}</button>
          ))}
        </div>
        <div className="segmented">
          <button className={sort === "cpu" ? "on" : ""} onClick={() => setSort("cpu")}>By CPU</button>
          <button className={sort === "memory" ? "on" : ""} onClick={() => setSort("memory")}>By memory</button>
        </div>
      </h4>
      {error && <p className="faint">{error}</p>}
      {!r && !error && <p className="faint">Measuring…</p>}
      {r && (
        <table className="table">
          <thead><tr><th>{view === "byService" ? "Service" : view === "byUser" ? "User" : "Program"}</th><th style={{ width: "28%" }}>CPU</th><th style={{ width: "28%" }}>Memory</th>{view === "processes" && <th />}</tr></thead>
          <tbody>
            {sorted.map((x) => (
              <tr key={x.key}>
                <td><span className="mono">{x.name}</span> <span className="faint" style={{ fontSize: 12 }}>{x.detail}</span></td>
                <td><div className="usage-cell"><Bar value={Math.min(100, (x.cpu / capacity) * 100)} hot={x.cpu / capacity > 0.8} /><span>{x.cpu.toFixed(1)}%</span></div></td>
                <td><div className="usage-cell"><Bar value={memTotal ? (x.rss / memTotal) * 100 : 0} hot={memTotal > 0 && x.rss / memTotal > 0.5} /><span>{bytes(x.rss)}</span></div></td>
                {view === "processes" && <td style={{ width: 70, textAlign: "right" }}>{x.pid !== null && <button className="btn ghost" style={{ height: 24 }} title="Stop this process (asks first)" onClick={() => stopProcess(s, x.pid!, x.name)}>Stop</button>}</td>}
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {r && view === "byService" && r.byService.length === 0 && <p className="faint">Services can only be told apart on Linux with systemd.</p>}
      {r && <p className="faint" style={{ margin: "8px 0 0", fontSize: 12 }}>CPU is in percent of one core; this server has {r.cores}, so up to {r.cores * 100}% in total. Memory is resident memory; shared pages count for every process using them.</p>}
    </section>
  );
}

/** Which folders take the disk space: scan one level, click to go deeper. */
function DiskByFolder({ s, mounts }: { s: SessionState; mounts: string[] }) {
  const showFolder = useStore((x) => x.showFolder);
  const [path, setPath] = useState<string | null>(null);
  const [u, setU] = useState<DiskUsage | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const scan = async (p: string) => {
    setPath(p);
    setLoading(true);
    setError(null);
    try {
      setU(await api.diskUsage(s.info.id, p));
    } catch (e) {
      setError(asApiError(e).message);
      setU(null);
    } finally {
      setLoading(false);
    }
  };
  const up = path && path !== "/" ? path.replace(/\/[^/]+\/?$/, "") || "/" : null;
  const max = u?.entries[0]?.bytes ?? 1;
  return (
    <section className="panel span-12">
      <h4>Disk usage by folder
        {path && <span className="mono faint">{path}</span>}
        <span style={{ flex: 1 }} />
        {up && <button className="btn" disabled={loading} onClick={() => scan(up)}><ArrowUp size={13} /> Up</button>}
        {path && <button className="btn" onClick={() => showFolder(s.info.id, path)} title="Open this folder in Files"><FolderOpen size={13} /> Open in Files</button>}
        {path && <button className="icon-btn" aria-label="Scan again" onClick={() => scan(path)}><RefreshCw size={13} className={loading ? "spin" : ""} /></button>}
      </h4>
      {!path && (
        <div className="chips">
          <span className="faint">Find what takes the space on</span>
          {[...new Set(["/", ...mounts])].map((m) => <button key={m} className="chip" onClick={() => scan(m)}>{m}</button>)}
        </div>
      )}
      {loading && <p className="faint">Adding up folder sizes under {path}… on a large disk this can take a minute.</p>}
      {error && <p className="bad-text">{error}</p>}
      {u && !loading && (
        <>
          <p className="faint" style={{ margin: "0 0 8px", fontSize: 12 }}>
            {bytes(u.total)} in {u.path}{u.fsSize ? ` · disk ${u.mount}: ${bytes(u.fsUsed)} used of ${bytes(u.fsSize)}, ${bytes(u.fsFree)} free` : ""}
            {u.partial && " · some folders couldn't be read (turn on sudo for exact numbers)"}
          </p>
          <table className="table">
            <tbody>
              {u.entries.slice(0, 30).map((e) => (
                <tr key={e.path} className={e.isDir ? "click" : ""} onClick={() => e.isDir && scan(e.path)} title={e.isDir ? "Look inside" : ""}>
                  <td style={{ width: 24 }}>{e.isDir ? <Folder size={14} color="var(--amber)" /> : <File size={14} className="faint" />}</td>
                  <td className="mono">{e.name}</td>
                  <td style={{ width: "40%" }}><div className="usage-cell"><Bar value={(e.bytes / max) * 100} hot={u.total > 0 && e.bytes / u.total > 0.5} /><span>{bytes(e.bytes)}</span></div></td>
                  <td className="num faint" style={{ width: 60 }}>{u.total ? `${Math.round((e.bytes / u.total) * 100)}%` : ""}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {u.entries.length === 0 && <p className="faint">Nothing large here.</p>}
          <p className="faint" style={{ margin: "8px 0 0", fontSize: 12 }}>Folders are added up on this disk only (other disks mounted inside are left out). Files under 1 MB are counted in their folder but not listed.</p>
        </>
      )}
    </section>
  );
}

/** Identity, network and housekeeping: things that change rarely, read once per visit. */
function ServerDetails({ s }: { s: SessionState }) {
  const setTab = useStore((x) => x.setTab);
  const { data: i, error, loading, reload } = useRemote(() => api.systemInfo(s.info.id), [s.info.id]);
  const dash = (v: unknown) => (v === null || v === undefined || v === "" ? "—" : String(v));
  return (
    <section className="panel span-12">
      <h4>Server details
        <button className="icon-btn" style={{ width: 22, height: 22, marginLeft: "auto" }} aria-label="Read details again" onClick={reload}><RefreshCw size={12} className={loading ? "spin" : ""} /></button>
      </h4>
      {error && <p className="faint">{error.message}</p>}
      {i && (
        <div className="details">
          <dl className="kv">
            <dt>Hostname</dt><dd>{dash(i.fqdn && i.fqdn !== i.hostname ? i.fqdn : i.hostname)}</dd>
            <dt>Operating system</dt><dd>{dash(i.os)}</dd>
            <dt>Kernel</dt><dd>{dash(i.kernel)} {i.arch && <span className="faint">· {i.arch}</span>}</dd>
            <dt>Machine</dt><dd>{dash(i.virtualization ? VIRT[i.virtualization] ?? i.virtualization : null)}</dd>
            <dt>Hardware</dt><dd>{dash([i.vendor, i.product].filter(Boolean).join(" ") || null)}</dd>
            <dt>CPU threads</dt><dd>{dash(i.cpuThreads)}</dd>
            <dt>Docker</dt><dd>{s.info.dockerVersion ?? "not installed"}</dd>
          </dl>
          <dl className="kv">
            <dt>Addresses</dt><dd>{i.addresses.length ? i.addresses.map((a) => <div key={a.interface + a.address} className="mono">{a.address} <span className="faint">{a.interface}</span></div>) : "—"}</dd>
            <dt>Gateway</dt><dd className="mono">{dash(i.gateway)}</dd>
            <dt>DNS servers</dt><dd className="mono">{i.dns.length ? i.dns.join(", ") : "—"}</dd>
            <dt>Time zone</dt><dd>{dash(i.timezone)}</dd>
            <dt>Started</dt><dd>{dash(i.bootTime)}</dd>
          </dl>
          <dl className="kv">
            <dt>Updates</dt>
            <dd>{i.updates == null ? <span className="faint">{i.packageManager ? `${i.packageManager}; count not available` : "—"}</span>
              : i.updates === 0 ? <span className="ok-text"><CheckCircle2 size={12} /> Up to date</span>
              : <span className={i.securityUpdates ? "bad-text" : "warn-text"}>{i.updates} pending{i.securityUpdates ? `, ${i.securityUpdates} security` : ""}</span>}</dd>
            <dt>Reboot</dt><dd>{i.rebootRequired ? <span className="warn-text"><AlertTriangle size={12} /> Needed to finish updates</span> : "Not needed"}</dd>
            <dt>Failed services</dt>
            <dd>{i.failedUnits.length === 0 ? <span className="ok-text"><CheckCircle2 size={12} /> None</span>
              : <a className="link bad-text" onClick={() => setTab(s.info.id, "services")}>{i.failedUnits.join(", ")}</a>}</dd>
            <dt>Logged in</dt><dd>{dash(i.usersLoggedIn)} sessions</dd>
            <dt>Processes</dt><dd>{dash(i.processes)}</dd>
            <dt>Protection</dt><dd>{dash(i.securityModule)}</dd>
          </dl>
        </div>
      )}
    </section>
  );
}

/** How much memory there is and where it goes, in plain words. */
function MemoryDetails({ m }: { m: Metrics }) {
  const d = m.memory;
  if (!d || !m.memTotal) return null;
  const pct = (v: number) => (v / m.memTotal) * 100;
  const kernel = d.slabUnreclaimable + d.kernelStack + d.pageTables;
  const reserved = d.installed != null ? d.installed - m.memTotal : null;
  const parts = [
    { key: "used", label: "Used by programs", value: d.used, hint: "Memory programs and the kernel are using now" },
    { key: "cache", label: "Cache and buffers", value: d.buffCache, hint: "Recently used files kept in memory to make them fast; given back as soon as programs need it" },
    { key: "free", label: "Free", value: d.free, hint: "Not used for anything" },
  ];
  const commitPct = d.commitLimit ? (d.committed / d.commitLimit) * 100 : 0;
  const row = (label: string, value: number, hint: string) => (
    <><dt title={hint}>{label}</dt><dd title={hint}>{bytes(value)} <span className="faint">{pct(value).toFixed(1)}%</span></dd></>
  );
  return (
    <section className="panel span-12 mem-details">
      <h4>Memory details</h4>
      <div className="mem-head">
        <div>
          <div className="faint" style={{ fontSize: 12 }}>{d.installed != null ? "Installed" : "Total"}</div>
          <div className="big">{bytes(d.installed ?? m.memTotal)}</div>
          <div className="faint" style={{ fontSize: 12 }}>
            {reserved != null && reserved > 0 ? <>{bytes(m.memTotal)} usable by Linux · {bytes(reserved)} reserved by the firmware and kernel</> : <>usable by Linux</>}
          </div>
        </div>
        <div>
          <div className="faint" style={{ fontSize: 12 }}>In use</div>
          <div className="big">{bytes(m.memUsed)}</div>
          <div className="faint" style={{ fontSize: 12 }}>{pct(m.memUsed).toFixed(0)}% of usable memory{m.memUsed - d.used > m.memTotal / 100 ? ` · programs ${bytes(d.used)}, plus ${bytes(m.memUsed - d.used)} of cache that can't be given back now` : ""}</div>
        </div>
        <div>
          <div className="faint" style={{ fontSize: 12 }}>Available for programs</div>
          <div className="big">{bytes(m.memAvailable)}</div>
          <div className="faint" style={{ fontSize: 12 }}>free memory plus cache Linux can give back</div>
        </div>
      </div>
      <div className="mem-stack" role="img" aria-label={parts.map((p) => `${p.label} ${bytes(p.value)}`).join(", ")}>
        {parts.map((p) => <i key={p.key} className={p.key} style={{ width: `${pct(p.value)}%` }} title={`${p.label}: ${bytes(p.value)}`} />)}
      </div>
      <div className="mem-legend">
        {parts.map((p) => <span key={p.key} title={p.hint}><i className={p.key} /> {p.label} <b>{bytes(p.value)}</b></span>)}
      </div>
      <div className="details">
        <dl className="kv">
          {row("Programs' own data", d.anon, "Heap and stacks of running programs (anonymous memory)")}
          {row("Shared and tmpfs", d.shared, "Shared memory and files in RAM disks such as /dev/shm and /run")}
          {row("Kernel", kernel, "Kernel data that can't be freed: unreclaimable slab, kernel stacks and page tables")}
          {d.hugePages > 0 && row("Huge pages", d.hugePages, "Reserved as huge pages (often for databases)")}
        </dl>
        <dl className="kv">
          {row("File cache", d.pageCache, "Contents of files read or written recently")}
          {row("Buffers", d.buffers, "Raw disk blocks and file system metadata")}
          {row("Reclaimable kernel cache", d.slabReclaimable, "Kernel caches such as directory entries and inodes, freed under pressure")}
          {row("Waiting to be written", d.dirty, "Changed file data not yet on disk")}
        </dl>
        <dl className="kv">
          <dt title="Memory programs have asked for (not all of it is used). The commit limit only stops them when the kernel's overcommit setting is strict (vm.overcommit_memory=2).">Promised to programs</dt>
          <dd>{bytes(d.committed)} <span className="faint">{commitPct.toFixed(0)}% of the {bytes(d.commitLimit)} commit limit</span></dd>
          <dt title="Recently used memory vs. memory the kernel would reclaim or swap out first">Active / inactive</dt>
          <dd>{bytes(d.active)} <span className="faint">/ {bytes(d.inactive)}</span></dd>
          <dt>Swap</dt>
          <dd>{m.swapTotal ? <>{bytes(m.swapUsed)} <span className="faint">of {bytes(m.swapTotal)}{d.swapCached ? ` · ${bytes(d.swapCached)} also in RAM` : ""}</span></> : <span className="faint">none</span>}</dd>
        </dl>
      </div>
      {m.memAvailable / m.memTotal < 0.1 && <p className="warn-text" style={{ margin: "10px 0 0", fontSize: 12 }}><AlertTriangle size={12} /> Less than 10% is available. Programs may slow down{m.swapTotal ? " as the server swaps" : ""}, and Linux may stop the largest one to free memory.</p>}
    </section>
  );
}

const WEEK = { preset: "7d" } as const;

/** Where the fullest disk is heading, at last week's pace (nothing when it isn't filling). */
function DiskForecast({ profileId }: { profileId: string }) {
  const week = useRangeHistory(profileId, 200, WEEK);
  const days = week ? diskForecast(week.points) : null;
  if (days === null) return null;
  const when = days < 2 ? `about ${Math.max(1, Math.round(days * 24))} hours` : `about ${Math.round(days)} days`;
  return <p className={`disk-forecast ${days < 7 ? "warn" : ""}`}>At last week's pace, the fullest disk is full in {when}.</p>;
}

/** "+1.2 GB/day", "shrinking", "steady", or nothing before 6 hours of data. Exported for tests. */
export function growthText(g: DiskSeries | undefined): string {
  if (!g || g.perDay === null) return "";
  const abs = Math.abs(g.perDay);
  if (abs < 1024 * 1024) return "steady";
  return `${g.perDay > 0 ? "+" : "−"}${bytes(abs)}/day`;
}

/** Each disk's last 7 days: growth and a small trend (refreshed every 10 minutes). */
function useDiskGrowth(profileId: string): Record<string, DiskSeries> {
  const [by, setBy] = useState<Record<string, DiskSeries>>({});
  useEffect(() => {
    const load = () => {
      const now = Math.floor(Date.now() / 1000);
      api.diskHistory(profileId, now - 7 * 86400, now, 60).then((l) => setBy(Object.fromEntries(l.map((x) => [x.mount, x]))), () => {});
    };
    load();
    const t = setInterval(load, 600_000);
    return () => clearInterval(t);
  }, [profileId]);
  return by;
}

export function MonitorView({ s }: { s: SessionState }) {
  const growth = useDiskGrowth(s.profile.id);
  const m = s.metrics[s.metrics.length - 1];
  if (!m) return <div className="empty"><p>Collecting the first sample…</p></div>;
  const hist = (f: (x: typeof m) => number) => s.metrics.map(f);
  const memPct = m.memTotal ? (m.memUsed / m.memTotal) * 100 : 0;
  const cachePct = m.memTotal ? (Math.min(m.memCached, m.memAvailable) / m.memTotal) * 100 : 0;
  const swapPct = m.swapTotal ? (m.swapUsed / m.swapTotal) * 100 : 0;

  return (
    <div className="monitor">
      <section className="panel span-4">
        <h4>CPU <span className="faint">{m.cores} cores</span></h4>
        <div className="big"><LiveNumber value={m.cpuPercent} suffix="%" /></div>
        <Sparkline values={hist((x) => x.cpuPercent ?? 0)} width={300} height={46} fluid />
        <div className="cores" aria-label="Per-core usage">
          {m.perCorePercent.map((v, i) => <div key={i} className="core" title={`Core ${i}: ${v.toFixed(0)}%`}><i style={{ height: `${v}%` }} /></div>)}
        </div>
        <p className="faint" style={{ margin: "10px 0 0", fontSize: 12 }}>{m.cpuModel || "Unknown CPU"}</p>
      </section>

      <section className="panel span-4">
        <h4>Memory <span className="faint">{bytes(m.memory?.installed ?? m.memTotal)}{m.memory?.installed ? " installed" : " total"}</span></h4>
        <div className="big">{bytes(m.memUsed)} <span className="faint" style={{ fontSize: 14 }}>of {bytes(m.memTotal)} used · {memPct.toFixed(0)}%</span></div>
        <Sparkline values={hist((x) => (x.memTotal ? (x.memUsed / x.memTotal) * 100 : 0))} width={300} height={46} fluid />
        <div style={{ marginTop: 12 }}><Bar value={memPct} cache={cachePct} hot={memPct > 90} /></div>
        <p className="faint" style={{ margin: "8px 0 0", fontSize: 12 }}>{bytes(m.memAvailable)} available · {bytes(m.memCached)} cache</p>
        <h4 style={{ marginTop: 14 }}>Swap <span className="faint">{m.swapTotal ? `${bytes(m.swapUsed)} of ${bytes(m.swapTotal)}` : "none"}</span></h4>
        {m.swapTotal > 0 && <Bar value={swapPct} hot={swapPct > 50} />}
        {m.swapTotal === 0 && isSsh(s) && <AddSwap s={s} memTotal={m.memTotal} />}
      </section>

      <section className="panel span-4">
        <h4>Load average</h4>
        <div className="big"><LiveNumber value={m.load[0]} decimals={2} /></div>
        <Sparkline values={hist((x) => x.load[0])} max={m.cores} width={300} height={46} fluid />
        <p className="faint" style={{ margin: "10px 0 0", fontSize: 12 }}>5 min {m.load[1].toFixed(2)} · 15 min {m.load[2].toFixed(2)} · {m.load[0] > m.cores ? "busier than its core count" : "within capacity"}</p>
        <h4 style={{ marginTop: 14 }}>Network</h4>
        <div style={{ display: "flex", gap: 24 }}>
          <div><div className="faint" style={{ fontSize: 12 }}>In</div><b>{rate(m.netRxPerSec)}</b></div>
          <div><div className="faint" style={{ fontSize: 12 }}>Out</div><b>{rate(m.netTxPerSec)}</b></div>
        </div>
      </section>

      <section className="panel span-12">
        <div className="panel-head"><h4>History</h4><RangePicker /></div>
        <HistoryChart profileId={s.profile.id} name={s.profile.name} />
      </section>

      <section className="panel span-7">
        <h4>Disks</h4>
        <DiskForecast profileId={s.profile.id} />
        <table className="table">
          <thead><tr><th>Mount</th><th>Device</th><th style={{ width: "30%" }}>Used</th><th className="num">Free</th><th title="Over the last 7 days">Growth</th></tr></thead>
          <tbody>
            {m.disks.map((d) => {
              const g = growth[d.mount];
              return (
                <tr key={d.mount + d.filesystem}>
                  <td className="mono">{d.mount}</td>
                  <td className="faint mono" style={{ fontSize: 11.5 }}>{d.filesystem} · {d.fsType}</td>
                  <td><div style={{ display: "flex", alignItems: "center", gap: 8 }}><Bar value={d.usedPercent} hot={d.usedPercent > 90} /><span style={{ fontSize: 12, minWidth: 34 }}>{d.usedPercent.toFixed(0)}%</span></div></td>
                  <td className="num">{bytes(d.availableBytes)}</td>
                  <td className="disk-growth">
                    {g && g.points.length > 1 && <Sparkline values={g.points.map((x) => x[1])} max={0} width={60} height={16} />}
                    <span className={g?.fullAt && g.fullAt - Date.now() / 1000 < 14 * 86400 ? "warn-ink" : "faint"} title={g?.fullAt ? `Full around ${new Date(g.fullAt * 1000).toLocaleDateString()} at this pace` : undefined}>{growthText(g)}</span>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </section>

      <section className="panel span-5">
        <h4>Uptime</h4>
        <div className="big">{duration(m.uptimeSecs)}</div>
        <p className="faint" style={{ margin: "6px 0 0", fontSize: 12 }}>{m.osName} · {m.kernel}</p>
        <p className="faint" style={{ margin: "4px 0 0", fontSize: 12 }}>init {s.info.hasSystemd ? "systemd" : "other"} · what uses CPU, memory and disk is below</p>
      </section>

      <MemoryDetails m={m} />

      <ServerDetails s={s} />

      <UsageByProgram s={s} memTotal={m.memTotal} />
      <DiskByFolder s={s} mounts={m.disks.map((d) => d.mount)} />
    </div>
  );
}

/** A size suggestion for swap, as in orbital_core::swap::suggest_mb. */
export function suggestSwapMb(memBytes: number): number {
  const mb = Math.floor(memBytes / (1024 * 1024));
  const raw = mb < 2048 ? mb * 2 : mb <= 8192 ? mb : 4096;
  return Math.min(8192, Math.max(1024, Math.ceil(raw / 256) * 256));
}

/** Servers without swap (common on small VPSs) can be given some, safely. */
function AddSwap({ s, memTotal }: { s: SessionState; memTotal: number }) {
  const [open, setOpen] = useState(false);
  const suggested = suggestSwapMb(memTotal);
  const [mb, setMb] = useState(suggested);
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const sizes = [...new Set([1024, 2048, 4096, 8192, suggested])].sort((a, b) => a - b);
  const go = async () => {
    setBusy(true);
    setProblem(null);
    try {
      await runJob((p) => api.addSwap(s.info.id, mb, p), { key: `swap:${s.info.id}`, title: `Adding ${mb / 1024} GB of swap`, sid: s.info.id, done: (b) => `Swap is on: ${bytes(b)}` });
      setOpen(false);
    } catch (e) {
      setProblem(asApiError(e).message.replace(/^Invalid input: /, ""));
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <p className="faint" style={{ margin: "6px 0 0", fontSize: 12 }}>
        No swap: when memory runs out, the kernel kills a program. <button className="link-btn" onClick={() => setOpen(true)}>Add swap…</button>
      </p>
      {open && (
        <Modal title="Add swap" subtitle="A swap file on the main disk, on now and after every reboot." onClose={() => setOpen(false)}
          footer={<><button className="btn" onClick={() => setOpen(false)}>Cancel</button><button className="btn primary" disabled={busy} onClick={go}>{busy ? "Adding…" : `Add ${mb / 1024} GB`}</button></>}>
          <div className="route-form">
            <div className="field">
              <span>Size</span>
              <div className="segmented" style={{ alignSelf: "flex-start" }}>
                {sizes.map((x) => <button key={x} className={mb === x ? "on" : ""} onClick={() => setMb(x)}>{x % 1024 ? `${(x / 1024).toFixed(2)} GB` : `${x / 1024} GB`}{x === suggested ? " ·  suggested" : ""}</button>)}
              </div>
              <span className="hint">Suggested from {bytes(memTotal)} of memory. Orbital checks first: nothing changes if swap is on already, /swapfile exists, or the disk hasn't room for it plus 1 GB.</span>
            </div>
            <p className="faint" style={{ margin: 0, fontSize: 12.5 }}>Creates /swapfile (root only), turns it on, adds it to /etc/fstab and sets vm.swappiness=10 so memory is used first.</p>
            {problem && <div className="alert">{problem}</div>}
          </div>
        </Modal>
      )}
    </>
  );
}
