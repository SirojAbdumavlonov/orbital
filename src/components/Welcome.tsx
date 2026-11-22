import { FileInput, Loader2, Plus } from "lucide-react";
import { isSsh, SessionState, useStore } from "../store";
import { Profile } from "../lib/api";
import { useSessionHealth } from "../lib/health";
import { duration, relativeTime } from "../lib/format";
// Bundled with the code (not fetched from /logo.svg), so it shows however the page is served.
import logo from "../assets/logo.svg";
import { WhatsNewCard } from "./WhatsNew";

export function Welcome() {
  const { profiles, setEditing, setImportingSsh } = useStore();
  return (
    <div className="welcome drag" data-tauri-drag-region>
      <div className={`welcome-inner ${profiles.length ? "with-overview" : ""}`} style={{ WebkitAppRegion: "no-drag" } as any}>
        <img className="welcome-logo" src={logo} width={56} height={56} alt="Orbital" />
        <h1>{profiles.length ? "Pick a server to open" : "Add your first server"}</h1>
        <p>
          Browse and edit files, restart services, manage containers and open a shell — over one SSH connection,
          with every change written to a tamper-evident activity log.
        </p>
        <div className="welcome-actions">
          <button className="btn primary" onClick={() => setEditing("new")}><Plus size={14} /> Add server</button>
          <button className="btn" onClick={() => setImportingSsh(true)}><FileInput size={14} /> Import from ~/.ssh/config</button>
        </div>
        {profiles.length > 0 && <Overview />}
        <WhatsNewCard />
      </div>
    </div>
  );
}

/** Every saved server at a glance: connected ones first with their numbers and anything that needs attention. */
function Overview() {
  const { profiles, sessions } = useStore();
  const live: Record<string, SessionState> = {};
  for (const x of Object.values(sessions)) if (!x.lost) live[x.profile.id] = x;
  const list = [...profiles].sort((a, b) => Number(!!live[b.id]) - Number(!!live[a.id]) || when(b) - when(a) || a.name.localeCompare(b.name));
  return (
    <div className="overview">
      <span className="faint overview-title">Your servers · {Object.keys(live).length} of {profiles.length} connected</span>
      <div className="overview-grid">
        {list.map((p) => <ServerCard key={p.id} p={p} s={live[p.id]} />)}
      </div>
    </div>
  );
}

function ServerCard({ p, s }: { p: Profile; s?: SessionState }) {
  const { connect, connecting } = useStore();
  const h = useSessionHealth(s?.info.id);
  const m = s?.metrics[s.metrics.length - 1];
  const disk = m?.disks.filter((d) => d.totalBytes > 64 * 1024 * 1024 && !/^(tmpfs|devtmpfs|overlay|squashfs)$/.test(d.fsType)).sort((a, b) => b.usedPercent - a.usedPercent)[0];
  const mem = m && m.memTotal ? Math.round(((m.memTotal - m.memAvailable) / m.memTotal) * 100) : null;
  const level = s ? h.level : "off";
  return (
    <button className={`server-card ${level}`} onClick={() => connect(p.id)} title={s ? `Open ${p.name}` : `Connect to ${p.name}`}>
      <div className="server-card-head">
        <span className={`dot ${s ? `live health-${h.level}` : ""}`} />
        <b>{p.name}</b>
        {connecting[p.id] && <Loader2 size={13} className="spin faint" />}
      </div>
      <span className="mono faint server-card-addr">{p.protocol === "s3" ? p.host : `${p.username}@${p.host}`}</span>
      {s && isSsh(s) && m ? (
        <div className="server-card-stats">
          <span><small>CPU</small>{m.cpuPercent === null ? "…" : `${Math.round(m.cpuPercent)}%`}</span>
          <span><small>Memory</small>{mem === null ? "…" : `${mem}%`}</span>
          <span><small>Disk{disk ? ` ${disk.mount}` : ""}</small>{disk ? `${Math.round(disk.usedPercent)}%` : "…"}</span>
          <span><small>Up</small>{duration(m.uptimeSecs)}</span>
        </div>
      ) : (
        <span className="faint server-card-note">
          {s ? (isSsh(s) ? "Connected · reading…" : "Connected (file storage)") : p.lastConnectedAt ? `Not connected · last opened ${relativeTime(when(p) / 1000)}` : "Not connected yet"}
        </span>
      )}
      {s && (h.level === "warn" || h.level === "bad") && (
        <div className={`server-card-reasons ${h.level}`}>{h.reasons.slice(0, 3).map((r) => <span key={r}>{r}</span>)}</div>
      )}
    </button>
  );
}

const when = (p: Profile) => (p.lastConnectedAt ? Date.parse(p.lastConnectedAt) : 0);
