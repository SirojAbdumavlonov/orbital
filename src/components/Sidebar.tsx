import { PROTOCOL_NAMES } from "./ProfileDialog";
import { Activity, ChevronLeft, ChevronRight, Globe, KeyRound, Loader2, Moon, PanelLeftClose, PanelLeftOpen, Pencil, Plus, Settings, Sun, Unplug } from "lucide-react";
import { isMac, KEYS } from "../lib/platform";
import { useResolvedTheme, useSettings } from "../lib/settings";
import { useMemo } from "react";
import { Profile } from "../lib/api";
import { useStore } from "../store";

export function Sidebar() {
  const { profiles, sessions, active, connecting, connect, disconnect, setEditing, globalView, setGlobalView } = useStore();
  const live = useMemo(() => {
    const m: Record<string, string> = {};
    for (const s of Object.values(sessions)) m[s.profile.id] = s.info.id;
    return m;
  }, [sessions]);
  const groups = useMemo(() => {
    const g = new Map<string, Profile[]>();
    for (const p of profiles) {
      const k = p.groupName?.trim() || "Servers";
      g.set(k, [...(g.get(k) ?? []), p]);
    }
    return [...g.entries()];
  }, [profiles]);
  const activeProfile = active ? sessions[active]?.profile.id : null;
  const theme = useResolvedTheme();
  const setPref = useSettings((x) => x.set);

  return (
    <aside className="sidebar">
      {/* The window's close/minimise/zoom buttons sit at the left of this bar. */}
      {/* The sidebar and back/forward buttons float over the left of this bar (NavButtons), so they stay put when the sidebar hides. */}
      <div className="sidebar-top" data-tauri-drag-region>
        <span className="spacer" data-tauri-drag-region />
        <button className="icon-btn" title={theme === "dark" ? "Switch to light mode" : "Switch to dark mode"} aria-label={theme === "dark" ? "Switch to light mode" : "Switch to dark mode"} onClick={() => setPref({ theme: theme === "dark" ? "light" : "dark" })}>
          {theme === "dark" ? <Sun size={15} /> : <Moon size={15} />}
        </button>
        <button className="icon-btn" title={`Add server (${KEYS.newServer})`} aria-label="Add server" onClick={() => setEditing("new")}>
          <Plus size={16} />
        </button>
      </div>
      <div className="sidebar-scroll">
        {groups.length === 0 && <p className="faint" style={{ padding: "8px" }}>No servers yet.</p>}
        {groups.map(([name, list]) => (
          <div key={name}>
            <div className="group-name">{name}</div>
            {list.map((p) => {
              const sid = live[p.id];
              return (
                <div
                  key={p.id}
                  className={`server ${activeProfile === p.id && !globalView ? "active" : ""}`}
                  onClick={() => connect(p.id)}
                  onDoubleClick={() => setEditing(p)}
                  title={`${p.username}@${p.host}:${p.port}`}
                >
                  <span className={`dot ${sid ? "live" : ""}`} style={!sid && p.color ? { background: p.color } : undefined} />
                  <div style={{ minWidth: 0 }}>
                    <div className="name">{p.name}</div>
                    <div className="addr">
                      {(p.protocol ?? "sftp") !== "sftp" && <span className="proto">{PROTOCOL_NAMES[p.protocol].replace(" (HTTPS)", "S")}</span>}
                      {p.protocol === "s3" ? p.initialPath || p.host : <>{p.username}@{p.host}{p.port !== 22 && (p.protocol ?? "sftp") === "sftp" ? `:${p.port}` : ""}</>}
                      {p.jumpProfileId && <span className="faint"> via {profiles.find((j) => j.id === p.jumpProfileId)?.name ?? "jump host"}</span>}
                    </div>
                  </div>
                  {connecting[p.id] ? (
                    <Loader2 size={14} className="spin faint" />
                  ) : (
                    <div className="row-actions">
                      <button className="icon-btn" aria-label="Edit server" title="Edit" onClick={(e) => { e.stopPropagation(); setEditing(p); }}>
                        <Pencil size={13} />
                      </button>
                      {sid && (
                        <button className="icon-btn" aria-label="Disconnect" title="Disconnect" onClick={(e) => { e.stopPropagation(); disconnect(sid); }}>
                          <Unplug size={13} />
                        </button>
                      )}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        ))}
      </div>
      <div className="sidebar-foot">
        <button className={`nav-item ${globalView === "domains" ? "on" : ""}`} onClick={() => setGlobalView("domains")}>
          <Globe size={15} /> Domains
        </button>
        <button className={`nav-item ${globalView === "activity" ? "on" : ""}`} onClick={() => setGlobalView("activity")}>
          <Activity size={15} /> Activity log
        </button>
        <button className={`nav-item ${globalView === "knownHosts" ? "on" : ""}`} onClick={() => setGlobalView("knownHosts")}>
          <KeyRound size={15} /> Security
        </button>
        <button className={`nav-item ${globalView === "settings" ? "on" : ""}`} onClick={() => setGlobalView("settings")}>
          <Settings size={15} /> Settings
        </button>
      </div>
    </aside>
  );
}

/**
 * Sidebar toggle and back/forward, next to the window buttons. Drawn once, at a
 * fixed place, whether the sidebar is shown or hidden, so they never move.
 */
export function NavButtons() {
  const { navBack, navFwd, goBack, goForward } = useStore();
  const hidden = useSettings((x) => x.sidebarHidden);
  const setPref = useSettings((x) => x.set);
  return (
    <div className="float-nav">
      <button className="icon-btn" title={`${hidden ? "Show" : "Hide"} sidebar (${KEYS.sidebar})`} aria-label={hidden ? "Show sidebar" : "Hide sidebar"} onClick={() => setPref({ sidebarHidden: !hidden })}>
        {hidden ? <PanelLeftOpen size={16} /> : <PanelLeftClose size={16} />}
      </button>
      <button className="icon-btn" title={`Back (${KEYS.back}${isMac ? " or swipe right with two fingers" : ""})`} aria-label="Back" disabled={navBack.length === 0} onClick={goBack}><ChevronLeft size={17} /></button>
      <button className="icon-btn" title={`Forward (${KEYS.forward})`} aria-label="Forward" disabled={navFwd.length === 0} onClick={goForward}><ChevronRight size={17} /></button>
    </div>
  );
}
