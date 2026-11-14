import { Plus } from "lucide-react";
import { useStore } from "../store";

export function Welcome() {
  const { profiles, setEditing, connect } = useStore();
  const recent = [...profiles].filter((p) => p.lastConnectedAt).sort((a, b) => (b.lastConnectedAt! > a.lastConnectedAt! ? 1 : -1)).slice(0, 5);
  return (
    <div className="welcome drag">
      <div className="welcome-inner" style={{ WebkitAppRegion: "no-drag" } as any}>
        <img src="/logo.svg" width={56} height={56} alt="" />
        <h1>{profiles.length ? "Pick a server to open" : "Add your first server"}</h1>
        <p>
          Browse and edit files, restart services, manage containers and open a shell — over one SSH connection,
          with every change written to a tamper-evident activity log.
        </p>
        <button className="btn primary" onClick={() => setEditing("new")}><Plus size={14} /> Add server</button>
        {recent.length > 0 && (
          <div className="recent">
            <span className="faint" style={{ fontSize: 12, marginBottom: 4 }}>Recently opened</span>
            {recent.map((p) => (
              <button key={p.id} onClick={() => connect(p.id)}>
                <span>{p.name}</span>
                <span className="mono faint">{p.username}@{p.host}</span>
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
