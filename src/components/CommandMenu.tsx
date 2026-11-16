import { Command } from "cmdk";
import { Activity, ArrowLeft, ArrowRight, Download, FolderTree, Globe, Gauge, KeyRound, ListChecks, Moon, Plug, Plus, RefreshCw, Server, Settings, SquareTerminal, Sun, Unplug } from "lucide-react";
import { useEffect, useState } from "react";
import { useResolvedTheme, useSettings } from "../lib/settings";
import { useUpdates } from "../lib/updates";
import { KEYS, mod } from "../lib/platform";
import { Tab, useStore } from "../store";
import { toolIcon } from "./ServerHeader";

const BASE_TABS: [Tab, string, typeof Gauge][] = [
  ["files", "Files", FolderTree],
  ["services", "Services", ListChecks],
  ["monitor", "Monitor", Gauge],
  ["terminal", "Terminal", SquareTerminal],
  ["activity", "Activity", Activity],
];

/** Every typed word must start a word of the item (so "ngi" finds nginx, not "staging"). */
function wordFilter(value: string, search: string, keywords?: string[]): number {
  const words = `${value} ${(keywords ?? []).join(" ")}`.toLowerCase().split(/[^a-z0-9]+/).filter(Boolean);
  const terms = search.toLowerCase().split(/\s+/).filter(Boolean);
  if (terms.length === 0) return 1;
  let score = 0;
  for (const t of terms) {
    const i = words.findIndex((w) => w.startsWith(t));
    if (i < 0) return 0;
    score += 1 / (1 + i);
  }
  return score;
}

/** ⌘K: jump to any server, tab or page, or run a common action, by typing. */
export function CommandMenu() {
  const [open, setOpen] = useState(false);
  const st = useStore();
  const theme = useResolvedTheme();
  const setPref = useSettings((x) => x.set);
  // The server in view, even while a page such as Settings covers it.
  const s = st.active ? st.sessions[st.active] : null;
  const go = (tab: Tab) => {
    if (!s) return;
    st.setActive(s.info.id);
    st.setTab(s.info.id, tab);
  };

  useEffect(() => {
    const h = (e: KeyboardEvent) => {
      if (mod(e) && e.key.toLowerCase() === "k" && !(e.target as Element | null)?.closest?.(".xterm")) {
        e.preventDefault();
        setOpen((o) => !o);
      }
    };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, []);

  const run = (f: () => void) => () => {
    setOpen(false);
    f();
  };

  return (
    <Command.Dialog filter={wordFilter} open={open} onOpenChange={setOpen} label="Command menu" overlayClassName="scrim cmdk-scrim" contentClassName="cmdk">
      <Command.Input placeholder="Go to a server, tab or page, or run an action…" />
      <Command.List>
        <Command.Empty>Nothing matches.</Command.Empty>
        {s && (
          <Command.Group heading={s.profile.name}>
            {BASE_TABS.filter(([id]) => (s.info.protocol !== "sftp" ? id === "files" || id === "activity" : id !== "services" || s.info.hasSystemd)).map(([id, label, Icon]) => (
              <Command.Item key={id} value={`tab ${label}`} onSelect={run(() => go(id))}><Icon size={14} /> {label}</Command.Item>
            ))}
            {s.toolTabs.map((id) => {
              const t = s.tools?.find((x) => x.id === id);
              const Icon = toolIcon(t);
              return <Command.Item key={id} value={`tab ${t?.name ?? id}`} onSelect={run(() => go(`tool:${id}`))}><Icon size={14} /> {t?.name ?? ({ ports: "Open ports", users: "Users", docker: "Docker", deploy: "Deploy", logs: "Logs", tunnels: "Tunnels" } as Record<string, string>)[id] ?? id}</Command.Item>;
            })}
            {(s.tools ?? []).filter((t) => !s.toolTabs.includes(t.id)).map((t) => {
              const Icon = toolIcon(t);
              return <Command.Item key={`open-${t.id}`} value={`open ${t.name} ${t.category}`} onSelect={run(() => { st.setActive(s.info.id); st.openTool(s.info.id, t.id); })}><Icon size={14} /> Open {t.name}<span className="cmdk-hint">{t.category}</span></Command.Item>;
            })}
            <Command.Item value="reconnect" onSelect={run(() => st.reconnect(s.info.id))}><RefreshCw size={14} /> Reconnect</Command.Item>
            <Command.Item value="disconnect" onSelect={run(() => st.disconnect(s.info.id))}><Unplug size={14} /> Disconnect</Command.Item>
          </Command.Group>
        )}
        <Command.Group heading="Servers">
          {st.profiles.map((p) => {
            const live = Object.values(st.sessions).some((x) => x.profile.id === p.id);
            return (
              <Command.Item key={p.id} value={`server ${p.name} ${p.host} ${p.username}`} onSelect={run(() => st.connect(p.id))}>
                {live ? <Server size={14} /> : <Plug size={14} />} {p.name}<span className="cmdk-hint mono">{p.username}@{p.host}</span>
              </Command.Item>
            );
          })}
          <Command.Item value="add server new" onSelect={run(() => st.setEditing("new"))}><Plus size={14} /> Add server…</Command.Item>
        </Command.Group>
        <Command.Group heading="Orbital">
          <Command.Item value="domains dns records cloudflare check" onSelect={run(() => st.setGlobalView("domains"))}><Globe size={14} /> Domains and DNS</Command.Item>
          <Command.Item value="activity log" onSelect={run(() => st.setGlobalView("activity"))}><Activity size={14} /> Activity log</Command.Item>
          <Command.Item value="security touch id hosts" onSelect={run(() => st.setGlobalView("knownHosts"))}><KeyRound size={14} /> Security</Command.Item>
          <Command.Item value="settings preferences" onSelect={run(() => st.setGlobalView("settings"))}><Settings size={14} /> Settings</Command.Item>
          <Command.Item value="check for updates new version" onSelect={run(() => { st.setGlobalView("settings"); void useUpdates.getState().check(); })}><Download size={14} /> Check for updates…</Command.Item>
          <Command.Item value="theme dark light mode" onSelect={run(() => setPref({ theme: theme === "dark" ? "light" : "dark" }))}>
            {theme === "dark" ? <Sun size={14} /> : <Moon size={14} />} Switch to {theme === "dark" ? "light" : "dark"} mode
          </Command.Item>
          <Command.Item value="back" onSelect={run(st.goBack)}><ArrowLeft size={14} /> Back<span className="cmdk-hint">{KEYS.back}</span></Command.Item>
          <Command.Item value="forward" onSelect={run(st.goForward)}><ArrowRight size={14} /> Forward<span className="cmdk-hint">{KEYS.forward}</span></Command.Item>
        </Command.Group>
      </Command.List>
    </Command.Dialog>
  );
}
