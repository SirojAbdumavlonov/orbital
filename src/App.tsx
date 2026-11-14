import { useEffect } from "react";
import { Prompts } from "./components/Prompts";
import { ProfileDialog } from "./components/ProfileDialog";
import { ServerHeader, useMetricsPoller } from "./components/ServerHeader";
import { Sidebar } from "./components/Sidebar";
import { Toasts } from "./components/Toasts";
import { ConfirmHost } from "./components/ui";
import { Welcome } from "./components/Welcome";
import { SessionState, useStore } from "./store";
import { ActivityView } from "./views/ActivityView";
import { DockerView } from "./views/DockerView";
import { FilesView } from "./views/FilesView";
import { KnownHostsView } from "./views/KnownHostsView";
import { MonitorView } from "./views/MonitorView";
import { ServicesView } from "./views/ServicesView";
import { TerminalView } from "./views/TerminalView";

/** One mounted workspace per open server. Files and terminals stay alive when hidden. */
function Workspace({ s, visible }: { s: SessionState; visible: boolean }) {
  useMetricsPoller(s, visible);
  return (
    <div className="main" style={{ display: visible ? "flex" : "none" }}>
      <ServerHeader s={s} />
      <div className="view" style={{ display: s.tab === "files" ? "flex" : "none" }}><FilesView s={s} /></div>
      <div className="view" style={{ display: s.tab === "terminal" ? "flex" : "none" }}><TerminalView s={s} /></div>
      {visible && s.tab === "services" && <ServicesView s={s} />}
      {visible && s.tab === "docker" && <DockerView s={s} />}
      {visible && s.tab === "monitor" && <MonitorView s={s} />}
      {visible && s.tab === "activity" && <ActivityView profileId={s.profile.id} />}
    </div>
  );
}

export default function App() {
  const { loadProfiles, order, sessions, active, globalView, setEditing, setGlobalView } = useStore();

  useEffect(() => {
    loadProfiles();
    const keys = (e: KeyboardEvent) => {
      if (e.metaKey && e.key === "n") {
        e.preventDefault();
        setEditing("new");
      }
    };
    window.addEventListener("keydown", keys);
    return () => window.removeEventListener("keydown", keys);
  }, []);

  const showWorkspace = !globalView && active;
  return (
    <div className="app">
      <Sidebar />
      {globalView && (
        <div className="main">
          <header className="header">
            <div className="header-top drag">
              <div className="header-title">
                <strong>{globalView === "activity" ? "Activity log" : "Trusted host keys"}</strong>
                <small>{globalView === "activity" ? "Every change across all servers, hash-chained so tampering is detectable" : "Host identities you've verified"}</small>
              </div>
              <button className="btn" style={{ marginLeft: "auto" }} onClick={() => setGlobalView(null)}>Close</button>
            </div>
          </header>
          {globalView === "activity" ? <ActivityView /> : <KnownHostsView />}
        </div>
      )}
      {order.map((id) => <Workspace key={id} s={sessions[id]} visible={!!showWorkspace && id === active} />)}
      {!globalView && !active && <div className="main"><Welcome /></div>}
      <ProfileDialog />
      <Prompts />
      <ConfirmHost />
      <Toasts />
    </div>
  );
}
