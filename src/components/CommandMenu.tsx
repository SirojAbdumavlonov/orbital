import { Command } from "cmdk";
import { Boxes, FileText, Activity, ArrowRightLeft, Cable, HeartPulse, SquareCode, ArrowLeft, ArrowRight, Download, FileInput, FolderTree, Globe, Gauge, Home, KeyRound, LifeBuoy, ListChecks, MessageSquare, Moon, Plug, Plus, RefreshCw, Server, Settings, Sparkles, SquareTerminal, Sun, Unplug, Video } from "lucide-react";
import { useEffect, useState } from "react";
import { useResolvedTheme, useSettings } from "../lib/settings";
import { useUpdates } from "../lib/updates";
import { KEYS, mod } from "../lib/platform";
import { goTo, Tab, useStore } from "../store";
import { toolIcon } from "./ServerHeader";
import { openItem, refreshIndex, SearchKind, useSearchIndex } from "../lib/searchIndex";

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

const SEARCH_ICONS: Record<SearchKind, typeof Server> = { service: ListChecks, container: Boxes, site: Globe, folder: FolderTree, file: FileText };

/** ⌘K: jump to any server, tab or page, or run a common action, by typing. */
export function CommandMenu() {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const st = useStore();
  const index = useSearchIndex();
  useEffect(() => {
    if (open) void refreshIndex();
    else setQuery("");
  }, [open]);
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
      <Command.Input value={query} onValueChange={setQuery} placeholder="Go to a server, tab or page, run an action, or find a service, container, site or file…" />
      <Command.List>
        <Command.Empty>{index.loading ? "Looking on your servers…" : "Nothing matches."}</Command.Empty>
        {query.trim().length >= 2 && index.items.length > 0 && (
          <Command.Group heading={`On your servers${index.loading ? " · updating…" : ""}`}>
            {index.items.map((it) => {
              const Icon = SEARCH_ICONS[it.kind];
              return (
                <Command.Item key={it.key} value={`${it.words} ${it.key}`} onSelect={run(() => openItem(it))}>
                  <Icon size={14} /> <span className="cmdk-label">{it.label}</span>
                  <span className="cmdk-hint">{it.server} · {it.detail}</span>
                </Command.Item>
              );
            })}
          </Command.Group>
        )}
        {s && (
          <Command.Group heading={s.profile.name}>
            {BASE_TABS.filter(([id]) => (s.info.protocol !== "sftp" ? id === "files" || id === "activity" : id !== "services" || s.info.hasSystemd)).map(([id, label, Icon]) => (
              <Command.Item key={id} value={`tab ${label}`} onSelect={run(() => go(id))}><Icon size={14} /> {label}</Command.Item>
            ))}
            {s.toolTabs.map((id) => {
              const t = s.tools?.find((x) => x.id === id);
              const Icon = toolIcon(t);
              return <Command.Item key={id} value={`tab ${t?.name ?? id}`} onSelect={run(() => go(`tool:${id}`))}><Icon size={14} /> {t?.name ?? ({ checkup: "Checkup", updates: "Updates", agent: "AI agent", cleanup: "Disk cleanup", ports: "Open ports", users: "Users", docker: "Docker", deploy: "Deploy", logs: "Logs", tunnels: "Port forwarding", sshkeys: "SSH keys", secaudit: "Security audit", kernel: "Kernel tuning" } as Record<string, string>)[id] ?? id}</Command.Item>;
            })}
            {(s.tools ?? []).filter((t) => !s.toolTabs.includes(t.id)).map((t) => {
              const Icon = toolIcon(t);
              return <Command.Item key={`open-${t.id}`} value={`open ${t.name} ${t.category}`} onSelect={run(() => { st.setActive(s.info.id); st.openTool(s.info.id, t.id); })}><Icon size={14} /> Open {t.name}<span className="cmdk-hint">{t.category}</span></Command.Item>;
            })}
            {s.tools?.some((t) => t.id === "nginx") && (
              <Command.Item value="new website site domain nginx https" onSelect={run(() => { st.setActive(s.info.id); st.openTool(s.info.id, "nginx"); st.setNewWebsite(s.info.id); })}><Globe size={14} /> New website…<span className="cmdk-hint">nginx</span></Command.Item>
            )}
            {s.info.protocol === "sftp" && (
              <Command.Item value="port forwarding forward tunnel localhost container" onSelect={run(() => { st.setActive(s.info.id); st.openTool(s.info.id, "tunnels"); })}><Cable size={14} /> Forward a port…</Command.Item>
            )}
            {s.info.protocol === "sftp" && (
              <Command.Item value="send copy file folder to another server transfer" onSelect={run(() => { st.setActive(s.info.id); st.setTab(s.info.id, "files"); useStore.setState({ sendRequest: s.info.id }); })}><ArrowRightLeft size={14} /> Send a file to another server…</Command.Item>
            )}
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
          <Command.Item value="import ssh config hosts" onSelect={run(() => st.setImportingSsh(true))}><FileInput size={14} /> Import from ~/.ssh/config…</Command.Item>
        </Command.Group>
        <Command.Group heading="Orbital">
          <Command.Item value="terminal recordings replay cast" onSelect={run(() => st.setGlobalView("recordings"))}><Video size={14} /> Terminal recordings</Command.Item>
          <Command.Item value="snippets saved commands run many servers" onSelect={run(() => st.setGlobalView("snippets"))}><SquareCode size={14} /> Snippets</Command.Item>
          <Command.Item value="domains dns records cloudflare check" onSelect={run(() => st.setGlobalView("domains"))}><Globe size={14} /> Domains and DNS</Command.Item>
          <Command.Item value="activity log" onSelect={run(() => st.setGlobalView("activity"))}><Activity size={14} /> Activity log</Command.Item>
          <Command.Item value="home main page start recent" onSelect={run(() => goTo(null))}><Home size={14} /> Home</Command.Item>
          <Command.Item value="uptime monitoring websites checks down" onSelect={run(() => st.setGlobalView("uptime"))}><HeartPulse size={14} /> Uptime</Command.Item>
          <Command.Item value="security touch id hosts" onSelect={run(() => st.setGlobalView("knownHosts"))}><KeyRound size={14} /> Security</Command.Item>
          <Command.Item value="settings preferences" onSelect={run(() => st.setGlobalView("settings"))}><Settings size={14} /> Settings</Command.Item>
          <Command.Item value="check for updates new version" onSelect={run(() => { st.setGlobalView("settings"); void useUpdates.getState().check(); })}><Download size={14} /> Check for updates…</Command.Item>
          <Command.Item value="what's new release notes changelog" onSelect={run(() => st.setGlobalView("changes"))}><Sparkles size={14} /> What's new</Command.Item>
          <Command.Item value="help getting started troubleshooting shortcuts" onSelect={run(() => st.setGlobalView("help"))}><LifeBuoy size={14} /> Help</Command.Item>
          <Command.Item value="feedback report problem bug idea suggestion complain" onSelect={run(() => st.setGlobalView("feedback"))}><MessageSquare size={14} /> Send feedback…</Command.Item>
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
