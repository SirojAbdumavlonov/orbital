import { PackageCheck, Stethoscope, ArrowLeftRight, Rocket, ScrollText, Users as UsersIcon, KeyRound } from "lucide-react";
import { Activity, Boxes, Cable, Clock, Cpu, Database, FolderTree, Gauge, Globe, ListChecks, LockKeyhole, Network, Plus, RefreshCw, ShieldCheck, SquareTerminal, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { api, asApiError, DetectedTool } from "../lib/api";
import { bytes, duration } from "../lib/format";
import { useSettings } from "../lib/settings";
import { ALWAYS_TOOLS, DEDICATED_TOOLS, isSsh, SessionState, Tab, useStore } from "../store";
import { PROTOCOL_NAMES } from "./ProfileDialog";
import { LiveNumber, Sparkline, StateDot } from "./ui";

const TABS: { id: Tab; label: string; icon: typeof Gauge }[] = [
  { id: "files", label: "Files", icon: FolderTree },
  { id: "services", label: "Services", icon: ListChecks },
  { id: "monitor", label: "Monitor", icon: Gauge },
  { id: "terminal", label: "Terminal", icon: SquareTerminal },
  { id: "activity", label: "Activity", icon: Activity },
];

const CATEGORY_ICON: Record<string, typeof Gauge> = {
  "Web server": Globe, "Load balancer": Network, Containers: Boxes, Database, Runtime: Cpu, Access: ShieldCheck, Scheduling: Clock, Certificates: LockKeyhole,
};
export const toolIcon = (t?: DetectedTool) => (t && CATEGORY_ICON[t.category]) || Boxes;

/** Tabs that exist on every server, with no detected software behind them. */
const BUILTIN: Record<string, { name: string; hint: string; icon: typeof Gauge }> = {
  checkup: { name: "Checkup", hint: "What needs attention on this server, and how to fix it", icon: Stethoscope },
  updates: { name: "Updates", hint: "Install package and security updates; reboot", icon: PackageCheck },
  deploy: { name: "Deploy", hint: "Pull, build and restart your app; roll back", icon: Rocket },
  logs: { name: "Logs", hint: "Follow log files, services and containers live", icon: ScrollText },
  tunnels: { name: "Tunnels", hint: "Use the server's ports from this computer", icon: ArrowLeftRight },
  ports: { name: "Open ports", hint: "What listens where, and who can reach it", icon: Cable },
  users: { name: "Users", hint: "Accounts, groups, sudo and SSH keys", icon: UsersIcon },
  sshkeys: { name: "SSH keys", hint: "Who can sign in with a key; make and install keys", icon: KeyRound },
};
const tabName = (id: string, t?: DetectedTool) => t?.name ?? BUILTIN[id]?.name ?? (id === "docker" ? "Docker" : id);

/** The "+" at the end of the tab strip: software found on this server that can be opened as a tab. */
function AddToolMenu({ s }: { s: SessionState }) {
  const { openTool, detectTools } = useStore();
  const [open, setOpen] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const away = (e: MouseEvent) => !ref.current?.contains(e.target as Node) && setOpen(false);
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    window.addEventListener("mousedown", away);
    window.addEventListener("keydown", esc);
    return () => {
      window.removeEventListener("mousedown", away);
      window.removeEventListener("keydown", esc);
    };
  }, [open]);
  const refresh = async () => {
    setRefreshing(true);
    await detectTools(s.info.id);
    setRefreshing(false);
  };
  // Only software Orbital has something to show for: its own view, or config and log files it found.
  const available = (s.tools ?? []).filter((t) => !s.toolTabs.includes(t.id) && (DEDICATED_TOOLS.includes(t.id) || t.paths.length > 0));
  // Open leftwards when the "+" sits too close to the window's right edge.
  const flip = open && (ref.current?.getBoundingClientRect().left ?? 0) + 300 > window.innerWidth;
  return (
    <div className="menu-anchor" ref={ref}>
      <button className="tab tab-add" aria-label="Add a tab for software on this server" aria-haspopup="menu" aria-expanded={open} title="Add a tab" onClick={() => { setOpen(!open); if (!open) refresh(); }}>
        <Plus size={15} />
      </button>
      {open && (
        <div className={`menu ${flip ? "flip" : ""}`} role="menu">
          <div className="menu-head">
            <span>Found on this server</span>
            <button className="icon-btn" style={{ width: 22, height: 22 }} aria-label="Look again" title="Look again" onClick={refresh}><RefreshCw size={12} className={refreshing ? "spin" : ""} /></button>
          </div>
          {s.tools === null && <div className="menu-empty">Looking…</div>}
          {s.tools !== null && available.length === 0 && (
            <div className="menu-empty">{s.tools.length === 0 ? "Nothing Orbital recognises yet." : "Everything found is already open."}</div>
          )}
          {ALWAYS_TOOLS.filter((id) => !s.toolTabs.includes(id)).map((id) => {
            const b = BUILTIN[id];
            return (
              <button key={id} role="menuitem" className="menu-item" onClick={() => { openTool(s.info.id, id); setOpen(false); }}>
                <b.icon size={15} />
                <span className="menu-name">{b.name}<small>{b.hint}</small></span>
              </button>
            );
          })}
          {available.map((t) => {
            const Icon = toolIcon(t);
            return (
              <button key={t.id} role="menuitem" className="menu-item" onClick={() => { openTool(s.info.id, t.id); setOpen(false); }}>
                <Icon size={15} />
                <span className="menu-name">{t.name}<small>{t.category}</small></span>
                <StateDot state={t.running ? "running" : "inactive"} label={t.running ? "running" : "stopped"} />
              </button>
            );
          })}
        </div>
      )}
    </div>
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
