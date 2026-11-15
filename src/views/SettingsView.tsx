import { Download, Monitor, Moon, RefreshCw, RotateCcw, Sun } from "lucide-react";
import { useEffect, useState } from "react";
import { confirm } from "../components/ui";
import { ThemePref, useSettings } from "../lib/settings";
import { bytes } from "../lib/format";
import { useUpdates } from "../lib/updates";
import { useStore } from "../store";

function Row({ title, hint, children }: { title: string; hint?: string; children: React.ReactNode }) {
  return (
    <div className="setting">
      <div className="setting-text">
        <b>{title}</b>
        {hint && <small>{hint}</small>}
      </div>
      <div className="setting-control">{children}</div>
    </div>
  );
}

function Stepper({ value, min, max, onChange, unit }: { value: number; min: number; max: number; onChange: (v: number) => void; unit: string }) {
  return (
    <div className="stepper">
      <button className="btn" aria-label="Smaller" disabled={value <= min} onClick={() => onChange(value - 1)}>−</button>
      <span className="mono">{value} {unit}</span>
      <button className="btn" aria-label="Larger" disabled={value >= max} onClick={() => onChange(value + 1)}>+</button>
    </div>
  );
}

/** Release notes are a CHANGELOG section: "### Heading" lines and "- item" bullets. Shown as text, never as HTML. */
function Notes({ text }: { text: string }) {
  const plain = (s: string) => s.replace(/\*\*(.+?)\*\*/g, "$1").replace(/`([^`]+)`/g, "$1");
  return (
    <div className="update-notes-body">
      {text.split("\n").map((l, i) => {
        const h = /^#{1,6}\s+(.*)/.exec(l);
        if (h) return <b key={i}>{plain(h[1])}</b>;
        const li = /^\s*[-*]\s+(.*)/.exec(l);
        if (li) return <div key={i} className="note-item">{plain(li[1])}</div>;
        return l.trim() ? <p key={i}>{plain(l)}</p> : null;
      })}
    </div>
  );
}

/** The current version, and a new one when there is one: check, download, restart. */
function UpdateRow({ version }: { version: string }) {
  const u = useUpdates();
  const pct = u.total ? Math.min(100, Math.round((u.got / u.total) * 100)) : null;
  const when = u.checkedAt ? new Date(u.checkedAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }) : null;
  const hint = {
    idle: "Updates are downloaded from Orbital's releases and checked against Orbital's signing key before they are installed.",
    checking: "Looking for a new version…",
    upToDate: `You have the latest version${when ? ` (checked at ${when})` : ""}.`,
    available: `Orbital ${u.version} is available. It is downloaded now and used the next time Orbital starts.`,
    downloading: `Downloading Orbital ${u.version}… ${pct !== null ? `${pct}%` : bytes(u.got)}`,
    ready: `Orbital ${u.version} is installed. Restart to use it; open connections and terminals are closed.`,
    error: `Couldn't check for updates: ${u.error ?? "unknown error"}`,
  }[u.status];
  return (
    <>
      <div className="setting">
        <div className="setting-text">
          <b>Orbital {version}</b>
          <small>{hint}</small>
          {u.status === "downloading" && <div className="progress"><span style={{ width: `${pct ?? 30}%` }} className={pct === null ? "indeterminate" : ""} /></div>}
        </div>
        <div className="setting-control">
          {u.status === "available" && <button className="btn primary" onClick={() => u.install()}><Download size={13} /> Update to {u.version}</button>}
          {u.status === "ready" && <button className="btn primary" onClick={() => u.restart()}><RefreshCw size={13} /> Restart now</button>}
          {!["available", "ready", "downloading"].includes(u.status) && (
            <button className="btn" disabled={u.status === "checking"} onClick={() => u.check()}>{u.status === "checking" ? "Checking…" : "Check for updates"}</button>
          )}
        </div>
      </div>
      {u.notes && (u.status === "available" || u.status === "ready") && (
        <div className="setting update-notes"><div className="setting-text"><b>What's new in {u.version}</b><Notes text={u.notes} /></div></div>
      )}
    </>
  );
}

/** Preferences for the app itself (this Mac), not for any server. */
export function SettingsView() {
  const st = useSettings();
  const { toast, setGlobalView } = useStore();
  const [version, setVersion] = useState("1.0.0");
  useEffect(() => {
    import("@tauri-apps/api/app").then((m) => m.getVersion()).then(setVersion).catch(() => {});
  }, []);

  async function forgetTabs() {
    const ok = await confirm({ title: "Forget remembered tabs?", body: "Each server opens with its default tabs next time you connect.", confirmLabel: "Forget tabs" });
    if (!ok) return;
    try {
      Object.keys(localStorage).filter((k) => k.startsWith("orbital.toolTabs.")).forEach((k) => localStorage.removeItem(k));
    } catch { /* nothing stored */ }
    toast({ kind: "success", title: "Remembered tabs cleared" });
  }

  const themes: [ThemePref, string, typeof Sun][] = [["system", "Match macOS", Monitor], ["light", "Light", Sun], ["dark", "Dark", Moon]];
  return (
    <div className="view">
      <div className="scroll">
        <div className="settings">
          <h3 className="settings-title">Appearance</h3>
          <Row title="Theme" hint="Match macOS follows System Settings, including automatic dark mode at night.">
            <div className="segmented">
              {themes.map(([id, label, Icon]) => (
                <button key={id} className={st.theme === id ? "on" : ""} onClick={() => st.set({ theme: id })}><Icon size={12} /> {label}</button>
              ))}
            </div>
          </Row>

          <h3 className="settings-title">Editor</h3>
          <Row title="Font size"><Stepper value={st.editorFontSize} min={10} max={22} unit="pt" onChange={(v) => st.set({ editorFontSize: v })} /></Row>
          <Row title="Wrap long lines" hint="Long lines continue on the next line instead of scrolling sideways.">
            <input type="checkbox" className="switch" checked={st.editorWordWrap} onChange={(e) => st.set({ editorWordWrap: e.target.checked })} />
          </Row>
          <Row title="Minimap" hint="A small overview of the whole file on the right.">
            <input type="checkbox" className="switch" checked={st.editorMinimap} onChange={(e) => st.set({ editorMinimap: e.target.checked })} />
          </Row>

          <h3 className="settings-title">Terminal</h3>
          <Row title="Open as root when sudo is set up" hint="New terminals start a root shell (sudo -i) on servers whose settings allow sudo, typing a saved sudo password for you. The other kind is one click away in the Terminal tab.">
            <input type="checkbox" className="switch" checked={st.terminalAsRoot} onChange={(e) => st.set({ terminalAsRoot: e.target.checked })} />
          </Row>
          <Row title="Font size"><Stepper value={st.terminalFontSize} min={10} max={22} unit="pt" onChange={(v) => st.set({ terminalFontSize: v })} /></Row>

          <h3 className="settings-title">Monitoring</h3>
          <Row title="Refresh every" hint="How often CPU, memory and network are sampled for the server in view. Servers in the background refresh every 10 seconds.">
            <div className="segmented">
              {[1, 2, 5, 10].map((n) => <button key={n} className={st.refreshSecs === n ? "on" : ""} onClick={() => st.set({ refreshSecs: n })}>{n} s</button>)}
            </div>
          </Row>

          <h3 className="settings-title">Updates</h3>
          <UpdateRow version={version} />
          <Row title="Check automatically" hint="Look for a new version when Orbital starts and every six hours. Nothing is installed until you click Update.">
            <input type="checkbox" className="switch" checked={st.autoUpdates} onChange={(e) => st.set({ autoUpdates: e.target.checked })} />
          </Row>

          <h3 className="settings-title">Security</h3>
          <Row title="Touch ID, trusted hosts" hint="Touch ID before saved passwords are used, and the host keys you have trusted.">
            <button className="btn" onClick={() => setGlobalView("knownHosts")}>Open Security</button>
          </Row>

          <h3 className="settings-title">Reset</h3>
          <Row title="Remembered tabs" hint="Which tool tabs each server opens with.">
            <button className="btn" onClick={forgetTabs}>Forget</button>
          </Row>
          <Row title="All preferences on this page">
            <button className="btn" onClick={() => st.reset()}><RotateCcw size={13} /> Restore defaults</button>
          </Row>

          <h3 className="settings-title">About</h3>
          <Row title={`Orbital ${version}`} hint="Servers, trusted host keys, the activity log and saved passwords (encrypted, readable only on this Mac) are kept in a hidden folder: ~/Library/Application Support/.orbital">
            <span className="faint">⌘K command menu · ⌘[ back · ⌘] forward</span>
          </Row>
        </div>
      </div>
    </div>
  );
}
