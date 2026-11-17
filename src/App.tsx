import { PanelRightClose } from "lucide-react";
import { useEffect } from "react";
import { Modal } from "./components/ui";
import { useNavigationGestures } from "./lib/navigation";
import { CommandMenu } from "./components/CommandMenu";
import { NewWebsiteDialog } from "./views/NewWebsite";
import { FolderPanel } from "./components/FolderPanel";
import { Prompts } from "./components/Prompts";
import { ProfileDialog } from "./components/ProfileDialog";
import { SshImportDialog } from "./components/SshImportDialog";
import { ServerHeader, useMetricsPoller } from "./components/ServerHeader";
import { NavButtons, Sidebar } from "./components/Sidebar";
import { Toasts } from "./components/Toasts";
import { ConfirmHost } from "./components/ui";
import { Welcome } from "./components/Welcome";
import { SessionState, useStore } from "./store";
import { ActivityView } from "./views/ActivityView";
import { DockerView } from "./views/DockerView";
import { DomainsView } from "./views/DomainsView";
import { EditorPane, FilesView } from "./views/FilesView";
import { KnownHostsView } from "./views/KnownHostsView";
import { SettingsView } from "./views/SettingsView";
import { useApplyTheme, useSettings } from "./lib/settings";
import { usePanelResizing } from "./lib/resize";
import { useUpdateChecks } from "./lib/updates";
import { isSidebarKey, mod } from "./lib/platform";
import { useUptimeAlerts } from "./lib/uptime";
import { UptimeView } from "./views/UptimeView";
import { MonitorView } from "./views/MonitorView";
import { NginxView } from "./views/NginxView";
import { ToolView } from "./views/ToolView";
import { CertbotView } from "./views/CertbotView";
import { Fail2banView } from "./views/Fail2banView";
import { FirewallView } from "./views/FirewallView";
import { PortsView } from "./views/PortsView";
import { SshdView } from "./views/SshdView";
import { DatabaseView, RedisView } from "./views/DatabaseView";
import { ApacheView, HaproxyView } from "./views/WebServerViews";
import { UsersView } from "./views/UsersView";
import { SshKeysView } from "./views/SshKeysView";
import { SnippetsView } from "./views/SnippetsView";
import { CronView } from "./views/CronView";
import { DeployView } from "./views/DeployView";
import { LogsView } from "./views/LogsView";
import { TunnelsView } from "./views/TunnelsView";
import { ServicesView } from "./views/ServicesView";
import { TerminalView } from "./views/TerminalView";

const GLOBAL_TITLES = {
  activity: ["Activity log", "Every change across all servers, hash-chained so tampering is detectable"],
  snippets: ["Snippets", "Saved commands, run on one server or many at once"],
  uptime: ["Uptime", "Websites and ports checked from this computer, with a notification when one goes down"],
  knownHosts: ["Security", "Saved credentials and the host identities you've verified"],
  settings: ["Settings", "Preferences for Orbital on this computer"],
  domains: ["Domains", "DNS records at your provider, and whether the world sees them yet"],
} as const;

/** Software with a screen of its own; everything else gets the generic tool tab. */
const TOOL_VIEWS: Record<string, (p: { s: SessionState }) => JSX.Element> = {
  docker: DockerView,
  nginx: NginxView,
  ufw: FirewallView,
  fail2ban: Fail2banView,
  certbot: CertbotView,
  sshd: SshdView,
  ports: PortsView,
  postgresql: ({ s }) => <DatabaseView s={s} toolId="postgresql" />,
  mysql: ({ s }) => <DatabaseView s={s} toolId="mysql" />,
  mongodb: ({ s }) => <DatabaseView s={s} toolId="mongodb" />,
  redis: RedisView,
  apache: ApacheView,
  haproxy: HaproxyView,
  users: UsersView,
  sshkeys: SshKeysView,
  cron: CronView,
  deploy: DeployView,
  logs: LogsView,
  tunnels: TunnelsView,
};

function ToolTab({ s, id }: { s: SessionState; id: string }) {
  const patch = useStore((x) => x.patch);
  const View = TOOL_VIEWS[id];
  const view = View ? <View key={id} s={s} /> : <ToolView key={id} s={s} toolId={id} />;
  // Files and folders opened from this tab show beside it, so the user stays on the tab.
  const editor = s.editorDock === s.tab && s.files.length > 0;
  const folder = s.folderDock?.tab === s.tab ? s.folderDock.path : null;
  const docked = editor || !!folder;
  // Same tree with or without the panel, so opening it doesn't reload the view.
  return (
    <div className={`with-dock ${docked ? "docked" : ""}`}>
      <div className="dock-main">{view}</div>
      {docked && <div className={`dock ${folder && editor ? "dock-both" : ""}`}>
        {folder && <FolderPanel s={s} path={folder} />}
        {editor && <>
          <div className="dock-head">
            <span className="faint">Editor</span>
            <span className="spacer" />
            <button className="btn ghost" onClick={() => patch(s.info.id, { tab: "files", editorDock: null })}>Open in Files</button>
            <button className="icon-btn" aria-label="Close the editor panel" title="Close the editor panel (files stay open in Files)" onClick={() => patch(s.info.id, { editorDock: null })}><PanelRightClose size={15} /></button>
          </div>
          <EditorPane s={s} />
        </>}
      </div>}
    </div>
  );
}

/** After connection settings of a connected server change: reconnect now, or later from the header. */
function ReconnectPrompt() {
  const { reconnectAsk, sessions, setReconnectAsk, reconnect } = useStore();
  const s = reconnectAsk ? sessions[reconnectAsk] : null;
  if (!s) return null;
  const busy = s.terminals.length > 0;
  return (
    <Modal title="Reconnect to apply the changes?" onClose={() => setReconnectAsk(null)}
      footer={<>
        <button className="btn" onClick={() => setReconnectAsk(null)}>Later</button>
        <button className="btn primary" autoFocus onClick={() => reconnect(s.info.id)}>Reconnect now</button>
      </>}>
      <p className="muted" style={{ margin: 0 }}>
        The connection settings of <b>{s.profile.name}</b> changed. The open connection still uses the old ones (for example the sudo setting), so they take effect after reconnecting.
        {busy && <> Open terminals will close.</>}
      </p>
    </Modal>
  );
}

/** One mounted workspace per open server. Files and terminals stay alive when hidden. */
function Workspace({ s, visible }: { s: SessionState; visible: boolean }) {
  useMetricsPoller(s, visible);
  return (
    <div className="main" style={{ display: visible ? "flex" : "none" }}>
      <ServerHeader s={s} />
      <div className="view" style={{ display: s.tab === "files" ? "flex" : "none" }}><FilesView s={s} /></div>
      <div className="view" style={{ display: s.tab === "terminal" ? "flex" : "none" }}><TerminalView s={s} /></div>
      {visible && s.tab === "services" && <ServicesView s={s} />}
      {visible && s.tab.startsWith("tool:") && <ToolTab s={s} id={s.tab.slice(5)} />}
      {visible && s.tab === "monitor" && <MonitorView s={s} />}
      {visible && s.tab === "activity" && <ActivityView profileId={s.profile.id} />}
    </div>
  );
}

export default function App() {
  const { loadProfiles, order, sessions, active, globalView, setEditing, setGlobalView, newWebsite } = useStore();
  useNavigationGestures();
  useApplyTheme();
  const sidebarHidden = useSettings((x) => x.sidebarHidden);
  usePanelResizing();
  useUpdateChecks();
  useUptimeAlerts();

  useEffect(() => {
    loadProfiles();
    const keys = (e: KeyboardEvent) => {
      // Ctrl shortcuts (Windows) mean something in a shell; the terminal keeps them.
      if ((e.target as Element | null)?.closest?.(".xterm")) return;
      // ⌃⌘S (Ctrl+Shift+S on Windows) hides and shows the sidebar, as in Finder.
      if (isSidebarKey(e)) {
        e.preventDefault();
        const st = useSettings.getState();
        st.set({ sidebarHidden: !st.sidebarHidden });
      }
      if (mod(e) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "n") {
        e.preventDefault();
        setEditing("new");
      }
    };
    window.addEventListener("keydown", keys);
    return () => window.removeEventListener("keydown", keys);
  }, []);

  const showWorkspace = !globalView && active;
  return (
    <>
    <div className={`app ${sidebarHidden ? "no-sidebar" : ""}`}>
      <Sidebar />
      <NavButtons />
      {globalView && (
        <div className="main">
          <header className="header">
            <div className="header-top drag" data-tauri-drag-region>
              <div className="header-title" data-tauri-drag-region>
                <strong>{GLOBAL_TITLES[globalView][0]}</strong>
                <small>{GLOBAL_TITLES[globalView][1]}</small>
              </div>
              <button className="btn" style={{ marginLeft: "auto" }} onClick={() => setGlobalView(null)}>Close</button>
            </div>
          </header>
          {globalView === "activity" ? <ActivityView /> : globalView === "uptime" ? <UptimeView /> : globalView === "snippets" ? <SnippetsView /> : globalView === "settings" ? <SettingsView /> : globalView === "domains" ? <DomainsView /> : <KnownHostsView />}
        </div>
      )}
      {order.map((id) => <Workspace key={id} s={sessions[id]} visible={!!showWorkspace && id === active} />)}
      {!globalView && !active && <div className="main"><Welcome /></div>}
      <ProfileDialog />
      <SshImportDialog />
      <Prompts />
      <ReconnectPrompt />
      <ConfirmHost />
      {newWebsite && sessions[newWebsite] && <NewWebsiteDialog sid={newWebsite} />}
    </div>
    {/* Outside the grid: these draw floating layers, not columns. */}
    <Toasts />
    <CommandMenu />
    </>
  );
}
