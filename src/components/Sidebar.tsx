import { Activity, KeyRound, Loader2, Pencil, Plus, Unplug } from "lucide-react";
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

  return (
    <aside className="sidebar">
      <div className="sidebar-top drag">
        <button className="icon-btn" title="Add server (⌘N)" aria-label="Add server" onClick={() => setEditing("new")}>
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
                    <div className="addr">{p.username}@{p.host}{p.port !== 22 ? `:${p.port}` : ""}</div>
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
        <button className={`nav-item ${globalView === "activity" ? "on" : ""}`} onClick={() => setGlobalView("activity")}>
          <Activity size={15} /> Activity log
        </button>
        <button className={`nav-item ${globalView === "knownHosts" ? "on" : ""}`} onClick={() => setGlobalView("knownHosts")}>
          <KeyRound size={15} /> Trusted host keys
        </button>
      </div>
    </aside>
  );
}
