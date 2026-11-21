import { Bot, HardDrive, PackageCheck, Stethoscope, ArrowLeftRight, Rocket, ScrollText, Users as UsersIcon, KeyRound, Radar, Download } from "lucide-react";
import { Activity, Boxes, Cable, Clock, Cpu, Database, FolderTree, Gauge, Globe, ListChecks, LockKeyhole, Network, Plus, RefreshCw, ShieldCheck, SquareTerminal, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { api, asApiError, DetectedTool, InstallOption } from "../lib/api";
import { runJob } from "../lib/jobs";
import { withSafetyOverride } from "../lib/remote";
import { bytes, duration } from "../lib/format";
import { useSettings } from "../lib/settings";
import { ALWAYS_TOOLS, DEDICATED_TOOLS, isSsh, SessionState, Tab, useStore } from "../store";
import { PROTOCOL_NAMES } from "./ProfileDialog";
import { confirm, LiveNumber, Modal, Sparkline, StateDot } from "./ui";

const TABS: { id: Tab; label: string; icon: typeof Gauge }[] = [
  { id: "files", label: "Files", icon: FolderTree },
  { id: "monitor", label: "Monitor", icon: Gauge },
  { id: "terminal", label: "Terminal", icon: SquareTerminal },
  { id: "services", label: "Services", icon: ListChecks },
  { id: "activity", label: "Activity", icon: Activity },
];

const CATEGORY_ICON: Record<string, typeof Gauge> = {
  "Web server": Globe, "Load balancer": Network, Containers: Boxes, Database, Runtime: Cpu, Access: ShieldCheck, Scheduling: Clock, Certificates: LockKeyhole,
};
export const toolIcon = (t?: DetectedTool) => (t && CATEGORY_ICON[t.category]) || Boxes;

/** Tabs that exist on every server, with no detected software behind them. */
const BUILTIN: Record<string, { name: string; hint: string; icon: typeof Gauge }> = {
  checkup: { name: "Checkup", hint: "What needs attention on this server, and how to fix it", icon: Stethoscope },
  agent: { name: "AI agent", hint: "Ask about this server from Telegram (Claude or ChatGPT), with alerts", icon: Bot },
  cleanup: { name: "Disk cleanup", hint: "Free space: old journal, package cache, Docker leftovers", icon: HardDrive },
  updates: { name: "Updates", hint: "Install package and security updates; reboot", icon: PackageCheck },
  deploy: { name: "Deploy", hint: "Pull, build and restart your app; roll back", icon: Rocket },
  logs: { name: "Logs", hint: "Follow log files, services and containers live", icon: ScrollText },
  tunnels: { name: "Port forwarding", hint: "Use a server port (or a container on 127.0.0.1) at localhost here", icon: ArrowLeftRight },
  ports: { name: "Open ports", hint: "What listens where, and who can reach it", icon: Cable },
  network: { name: "Network check", hint: "Can the server reach a host, port or URL? Ping, DNS, port, HTTP, traceroute", icon: Radar },
  users: { name: "Users", hint: "Accounts, groups, sudo and SSH keys", icon: UsersIcon },
  sshkeys: { name: "SSH keys", hint: "Who can sign in with a key; make and install keys", icon: KeyRound },
};
const tabName = (id: string, t?: DetectedTool) => t?.name ?? BUILTIN[id]?.name ?? (id === "docker" ? "Docker" : id);

/**
 * The "+" at the end of the tab strip: a window listing everything that can
 * open as a tab here, the software found on this server and the tools every
 * server has, searchable and scrollable.
 */
function AddToolMenu({ s }: { s: SessionState }) {
  const { openTool, detectTools } = useStore();
  const [open, setOpen] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [q, setQ] = useState("");
  const [installable, setInstallable] = useState<InstallOption[]>([]);
  const [installing, setInstalling] = useState<string | null>(null);
  useEffect(() => { if (open && installable.length === 0) api.installOptions().then(setInstallable).catch(() => {}); }, [open]);
  const refresh = async () => {
    setRefreshing(true);
    await detectTools(s.info.id);
    setRefreshing(false);
  };
  // Only software Orbital has something to show for: its own view, or config and log files it found.
  const found = (s.tools ?? []).filter((t) => !s.toolTabs.includes(t.id) && (DEDICATED_TOOLS.includes(t.id) || t.paths.length > 0));
  const builtin = ALWAYS_TOOLS.filter((id) => !s.toolTabs.includes(id));
  const words = q.toLowerCase().split(/\s+/).filter(Boolean);
  const hit = (...texts: string[]) => words.every((w) => texts.some((t) => t.toLowerCase().includes(w)));
  const foundShown = found.filter((t) => hit(t.name, t.category, t.id));
  const builtinShown = builtin.filter((id) => hit(BUILTIN[id].name, BUILTIN[id].hint, id));
  const pick = (id: string) => {
    openTool(s.info.id, id);
    setOpen(false);
    setQ("");
  };
  // Supported software that isn't on this server (in a container counts as there).
  const tryShown = isSsh(s) && s.tools !== null
    ? installable.filter((o) => !(s.tools ?? []).some((t) => t.id === o.id) && hit(o.name, o.category, o.hint, o.id))
    : [];
  async function install(o: InstallOption) {
    const ok = await confirm({
      title: `Install ${o.name} on ${s.profile.name}?`,
      body: `From the server's own package repositories (apt: ${o.apt.join(" ")}; dnf: ${o.dnf.join(" ")}), then started${o.id === "ufw" ? " (ufw stays off until you turn it on in its tab)" : ""}. It takes a minute or two; you can keep working.`,
      confirmLabel: "Install",
    });
    if (!ok) return;
    setInstalling(o.id);
    try {
      const done = await withSafetyOverride(
        (force) => runJob((p) => api.installSoftware(s.info.id, o.id, force, p), { key: `install:${s.info.id}:${o.id}`, title: `Installing ${o.name}`, sid: s.info.id, done: () => `${o.name} is installed on ${s.profile.name}` }),
        `${o.name} needs port 80`,
      );
      if (done) {
        await detectTools(s.info.id);
        pick(o.id);
      }
    } catch {
      // The job reports the error.
    } finally {
      setInstalling(null);
    }
  }
  const first = foundShown[0]?.id ?? builtinShown[0];
  return (
    <>
      <button className="tab tab-add" aria-label="Add a tab for software on this server" aria-haspopup="dialog" title="Add a tab" onClick={() => { setOpen(true); refresh(); }}>
        <Plus size={15} />
      </button>
      {open && (
        <Modal title="Add a tab" subtitle={<>Tools for <b>{s.profile.name}</b>: what Orbital found on it, and what every server has.</>} wide onClose={() => { setOpen(false); setQ(""); }}>
          <div className="picker-search">
            <input className="input" autoFocus placeholder="Search: docker, logs, firewall, database…" value={q} onChange={(e) => setQ(e.target.value)} onKeyDown={(e) => e.key === "Enter" && first && pick(first)} />
            <button className="icon-btn" aria-label="Look again" title="Look for software again" onClick={refresh}><RefreshCw size={14} className={refreshing ? "spin" : ""} /></button>
          </div>
          <div className="picker">
            <div className="picker-title">Found on this server{s.tools === null ? " · looking…" : ""}</div>
            {s.tools !== null && foundShown.length === 0 && (
              <div className="faint picker-empty">{q ? "Nothing found on this server matches." : s.tools.length === 0 ? "Nothing Orbital recognises yet." : "Everything found is already open."}</div>
            )}
            <div className="picker-grid">
              {foundShown.map((t) => {
                const Icon = toolIcon(t);
                return (
                  <button key={t.id} className="picker-card" onClick={() => pick(t.id)}>
                    <Icon size={17} />
                    <span className="picker-text"><b>{t.name}</b><small>{t.category}</small></span>
                    <StateDot state={t.running ? "running" : "inactive"} label={t.running ? "running" : "stopped"} />
                  </button>
                );
              })}
            </div>
            {tryShown.length > 0 && (
              <>
                <div className="picker-title">You may try <span className="faint">· not on this server yet; Orbital installs and starts it</span></div>
                <div className="picker-grid">
                  {tryShown.map((o) => (
                    <button key={o.id} className="picker-card try" disabled={!!installing} onClick={() => install(o)} title={`Install ${o.name} (${o.apt.join(" ")})`}>
                      <Download size={17} className={installing === o.id ? "spin" : ""} />
                      <span className="picker-text"><b>{o.name}</b><small>{o.hint}</small></span>
                      <span className="badge">{installing === o.id ? "installing…" : "Install"}</span>
                    </button>
                  ))}
                </div>
              </>
            )}
            <div className="picker-title">Tools for every server</div>
            {builtinShown.length === 0 && <div className="faint picker-empty">{q ? "No tool matches." : "All of them are open."}</div>}
            <div className="picker-grid">
              {builtinShown.map((id) => {
                const b = BUILTIN[id];
                return (
                  <button key={id} className="picker-card" onClick={() => pick(id)}>
                    <b.icon size={17} />
                    <span className="picker-text"><b>{b.name}</b><small>{b.hint}</small></span>
                  </button>
                );
              })}
            </div>
          </div>
        </Modal>
      )}
    </>
  );
}

/** Polls metrics for every open session (faster for the one in view). */
export function useMetricsPoller(s: SessionState, visible: boolean) {
  const pushMetrics = useStore((x) => x.pushMetrics);
  const patch = useStore((x) => x.patch);
  const toast = useStore((x) => x.toast);
  const ssh = isSsh(s);
  useEffect(() => {
    // File-storage connections have no server to measure.
    if (!ssh) return;
    let stop = false;
    let timer: number;
    const tick = async () => {
      try {
        pushMetrics(s.info.id, await api.metrics(s.info.id));
      } catch (e) {
        const err = asApiError(e);
        if (err.kind === "NotConnected") {
          patch(s.info.id, { lost: true });
          toast({ kind: "error", title: `Connection to ${s.profile.name} was lost` });
          return;
        }
      }
      if (!stop) timer = window.setTimeout(tick, visible ? useSettings.getState().refreshSecs * 1000 : 10000);
    };
    tick();
    return () => {
      stop = true;
      clearTimeout(timer);
    };
  }, [s.info.id, visible]);
}

export function ServerHeader({ s }: { s: SessionState }) {
  const { setTab, order, sessions, active, setActive, disconnect, connect, closeTool, reconnect } = useStore();
  const last = s.metrics[s.metrics.length - 1];
  const cpuHist = s.metrics.map((m) => m.cpuPercent ?? 0).slice(-40);
  const memHist = s.metrics.map((m) => (m.memTotal ? (m.memUsed / m.memTotal) * 100 : 0)).slice(-40);
  const memPct = last && last.memTotal ? (last.memUsed / last.memTotal) * 100 : null;
  const strip = useRef<HTMLDivElement>(null);
  // Keep the selected tab visible when the strip scrolls.
  useEffect(() => { strip.current?.querySelector(".tab.on")?.scrollIntoView({ block: "nearest", inline: "nearest" }); }, [s.tab, s.toolTabs.length]);

  return (
    <header className="header">
      {order.length > 1 && (
        <div className="session-strip drag" data-tauri-drag-region>
          {order.map((id) => (
            <div key={id} className={`session-chip ${id === active ? "on" : ""}`} onClick={() => setActive(id)}>
              {sessions[id].profile.name}
              <button className="icon-btn" style={{ width: 20, height: 20 }} aria-label="Disconnect" onClick={(e) => { e.stopPropagation(); disconnect(id); }}>
                <X size={12} />
              </button>
            </div>
          ))}
        </div>
      )}
      <div className="header-top drag" data-tauri-drag-region>
        <div className="header-title" data-tauri-drag-region>
          <strong>{s.profile.name}</strong>
          <small className="selectable">
            {s.info.label}
            {s.info.via.length > 0 ? `  ·  via ${s.info.via.map((v) => v.replace(/:22$/, "")).join(" → ")}` : ""}
            {last ? `  ·  ${last.osName}  ·  up ${duration(last.uptimeSecs)}` : ""}
          </small>
        </div>
        {s.lost ? (
          <div className="signal">
            <span className="badge bad">Disconnected</span>
            <button className="btn primary" onClick={() => { disconnect(s.info.id); connect(s.profile.id); }}>Reconnect</button>
          </div>
        ) : !isSsh(s) ? (
          <div className="signal">
            <span className="badge">{PROTOCOL_NAMES[s.info.protocol]}</span>
          </div>
        ) : (
          <div className="signal" aria-label="Live server load">
            <div className="signal-item" title={last?.cpuModel}>
              <b><LiveNumber value={last?.cpuPercent} suffix="%" /></b>
              <Sparkline values={cpuHist} />
              <span>CPU · {last?.cores ?? "–"} cores</span>
            </div>
            <div className="signal-item">
              <b><LiveNumber value={memPct} suffix="%" /></b>
              <Sparkline values={memHist} />
              <span title="Used by programs, of the memory Linux can use">Memory · {last ? `${bytes(last.memUsed)} of ${bytes(last.memTotal, 0)}` : "–"}</span>
            </div>
            <div className="signal-item">
              <b><LiveNumber value={last?.load[0]} decimals={2} /></b>
              <span style={{ gridRow: "span 2" }}>Load 1m</span>
            </div>
          </div>
        )}
      </div>
      {s.stale && !s.lost && (
        <div className="stale-bar" role="status">
          <span>Server settings changed. Reconnect to use them.</span>
          <span className="spacer" />
          <button className="btn primary" onClick={() => reconnect(s.info.id)}>Reconnect</button>
        </div>
      )}
      <nav className="tabs" role="tablist">
        <div className="tab-scroll" ref={strip}>
        {TABS.filter((t) => (!isSsh(s) ? t.id === "files" || t.id === "activity" : t.id === "services" ? s.info.hasSystemd : true)).map((t) => (
          <button key={t.id} role="tab" aria-selected={s.tab === t.id} className={`tab ${s.tab === t.id ? "on" : ""}`} onClick={() => setTab(s.info.id, t.id)}>
            <t.icon size={15} />
            {t.label}
            {t.id === "terminal" && s.terminals.length > 0 && <span className="count">{s.terminals.length}</span>}
            {t.id === "files" && s.files.some((f) => f.content !== f.original) && <span className="count">●</span>}
          </button>
        ))}
        {s.toolTabs.map((id) => {
          const tool = s.tools?.find((t) => t.id === id);
          const Icon = BUILTIN[id]?.icon ?? toolIcon(tool);
          const tab = `tool:${id}` as Tab;
          return (
            <button key={id} role="tab" aria-selected={s.tab === tab} className={`tab ${s.tab === tab ? "on" : ""}`} onClick={() => setTab(s.info.id, tab)}>
              <Icon size={15} />
              {tabName(id, tool)}
              <span className="tab-close" role="button" aria-label={`Close ${tabName(id, tool)} tab`} title="Close tab" onClick={(e) => { e.stopPropagation(); closeTool(s.info.id, id); }}>
                <X size={11} />
              </span>
            </button>
          );
        })}
        </div>
        {isSsh(s) && <AddToolMenu s={s} />}
      </nav>
    </header>
  );
}
