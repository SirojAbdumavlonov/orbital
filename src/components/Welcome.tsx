import { FileInput, Plus } from "lucide-react";
import { useStore } from "../store";
// Bundled with the code (not fetched from /logo.svg), so it shows however the page is served.
import logo from "../assets/logo.svg";
import { WhatsNewCard } from "./WhatsNew";

export function Welcome() {
  const { profiles, setEditing, setImportingSsh, connect } = useStore();
  const recent = [...profiles].filter((p) => p.lastConnectedAt).sort((a, b) => (b.lastConnectedAt! > a.lastConnectedAt! ? 1 : -1)).slice(0, 5);
  return (
    <div className="welcome drag" data-tauri-drag-region>
      <div className="welcome-inner" style={{ WebkitAppRegion: "no-drag" } as any}>
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
        <WhatsNewCard />
      </div>
    </div>
  );
}
