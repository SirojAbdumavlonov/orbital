import { Activity, Boxes, FolderTree, Gauge, ListChecks, SquareTerminal, X } from "lucide-react";
import { useEffect } from "react";
import { api, asApiError } from "../lib/api";
import { bytes, duration, percent } from "../lib/format";
import { SessionState, Tab, useStore } from "../store";
import { Sparkline } from "./ui";

const TABS: { id: Tab; label: string; icon: typeof Gauge }[] = [
  { id: "files", label: "Files", icon: FolderTree },
  { id: "services", label: "Services", icon: ListChecks },
  { id: "docker", label: "Docker", icon: Boxes },
  { id: "monitor", label: "Monitor", icon: Gauge },
  { id: "terminal", label: "Terminal", icon: SquareTerminal },
  { id: "activity", label: "Activity", icon: Activity },
];

/** Polls metrics for every open session (faster for the one in view). */
export function useMetricsPoller(s: SessionState, visible: boolean) {
  const pushMetrics = useStore((x) => x.pushMetrics);
  const patch = useStore((x) => x.patch);
  const toast = useStore((x) => x.toast);
  useEffect(() => {
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
      if (!stop) timer = window.setTimeout(tick, visible ? 2000 : 10000);
    };
    tick();
    return () => {
      stop = true;
      clearTimeout(timer);
    };
  }, [s.info.id, visible]);
}

export function ServerHeader({ s }: { s: SessionState }) {
  const { setTab, order, sessions, active, setActive, disconnect, connect } = useStore();
  const last = s.metrics[s.metrics.length - 1];
  const cpuHist = s.metrics.map((m) => m.cpuPercent ?? 0).slice(-40);
  const memHist = s.metrics.map((m) => (m.memTotal ? (m.memUsed / m.memTotal) * 100 : 0)).slice(-40);
  const memPct = last && last.memTotal ? (last.memUsed / last.memTotal) * 100 : null;

  return (
    <header className="header">
      {order.length > 1 && (
        <div className="session-strip drag">
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
      <div className="header-top drag">
        <div className="header-title">
          <strong>{s.profile.name}</strong>
          <small className="selectable">
            {s.info.label}
            {last ? `  ·  ${last.osName}  ·  up ${duration(last.uptimeSecs)}` : ""}
          </small>
        </div>
        {s.lost ? (
          <div className="signal">
            <span className="badge bad">Disconnected</span>
            <button className="btn primary" onClick={() => { disconnect(s.info.id); connect(s.profile.id); }}>Reconnect</button>
          </div>
        ) : (
          <div className="signal" aria-label="Live server load">
            <div className="signal-item" title={last?.cpuModel}>
              <b>{percent(last?.cpuPercent)}</b>
              <Sparkline values={cpuHist} />
              <span>CPU · {last?.cores ?? "–"} cores</span>
            </div>
            <div className="signal-item">
              <b>{percent(memPct)}</b>
              <Sparkline values={memHist} />
              <span>Memory · {bytes(last?.memTotal, 0)}</span>
            </div>
            <div className="signal-item">
              <b>{last ? last.load[0].toFixed(2) : "—"}</b>
              <span style={{ gridRow: "span 2" }}>Load 1m</span>
            </div>
          </div>
        )}
      </div>
      <nav className="tabs" role="tablist">
        {TABS.filter((t) => (t.id === "services" ? s.info.hasSystemd : true)).map((t) => (
          <button key={t.id} role="tab" aria-selected={s.tab === t.id} className={`tab ${s.tab === t.id ? "on" : ""}`} onClick={() => setTab(s.info.id, t.id)}>
            <t.icon size={15} />
            {t.label}
            {t.id === "terminal" && s.terminals.length > 0 && <span className="count">{s.terminals.length}</span>}
            {t.id === "files" && s.files.some((f) => f.content !== f.original) && <span className="count">●</span>}
          </button>
        ))}
      </nav>
    </header>
  );
}
