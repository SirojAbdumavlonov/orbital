import { create } from "zustand";
import { api } from "./api";
import { moveTo } from "./kept";
import { isSsh, SessionState, useStore } from "../store";

// "Search everywhere" (⌘K): services, containers, nginx sites, bookmarked
// folders and open files across every connected server. Read when the menu
// opens (at most every 30 s); choosing one opens it in place, with that item
// selected (through the views' kept state).

export type SearchKind = "service" | "container" | "site" | "folder" | "file";

export interface SearchItem {
  kind: SearchKind;
  key: string;
  label: string;
  detail: string;
  sid: string;
  server: string;
  /** Words cmdk matches against. */
  words: string;
}

const FRESH_MS = 30_000;

export const useSearchIndex = create<{ items: SearchItem[]; loading: boolean; at: number }>(() => ({ items: [], loading: false, at: 0 }));

function bookmarks(profileId: string): string[] {
  try {
    const v = JSON.parse(localStorage.getItem(`orbital.bookmarks.${profileId}`) ?? "[]");
    return Array.isArray(v) ? v.filter((x) => typeof x === "string") : [];
  } catch {
    return [];
  }
}

async function forSession(s: SessionState): Promise<SearchItem[]> {
  const sid = s.info.id;
  const server = s.profile.name;
  const item = (kind: SearchKind, key: string, label: string, detail: string): SearchItem => ({
    kind, key: `${sid}:${kind}:${key}`, label, detail, sid, server, words: `${kind} ${label} ${detail} ${server}`,
  });
  const out: SearchItem[] = [];
  for (const f of s.files) out.push(item("file", f.path, f.path.split("/").pop() ?? f.path, f.path));
  for (const b of bookmarks(s.profile.id)) out.push(item("folder", b, b, "bookmarked folder"));
  if (!isSsh(s)) return out;
  const has = (id: string) => !!s.tools?.some((t) => t.id === id);
  const jobs: Promise<void>[] = [];
  if (s.info.hasSystemd) {
    jobs.push(api.listServices(sid).then((l) => {
      for (const x of l) out.push(item("service", x.unit, x.unit.replace(/\.service$/, ""), `${x.active}${x.description ? ` · ${x.description}` : ""}`));
    }, () => {}));
  }
  if (has("docker") || s.info.dockerVersion) {
    jobs.push(api.listContainers(sid).then((l) => {
      for (const c of l) out.push(item("container", c.id, c.names, `${c.image} · ${c.state}`));
    }, () => {}));
  }
  if (has("nginx")) {
    jobs.push(api.nginxConfig(sid).then((cfg) => {
      for (const site of cfg.sites) {
        if (!site.serverNames.length) continue;
        out.push(item("site", `${site.file}:${site.line}`, site.serverNames.join(", "), `nginx${site.ssl ? " · HTTPS" : ""} · ${site.file.replace(/^\/etc\/nginx\//, "")}`));
      }
    }, () => {}));
  }
  await Promise.all(jobs);
  return out;
}

/** Reads every connected server again, unless that was done in the last 30 seconds. */
export async function refreshIndex(force = false) {
  const { at, loading } = useSearchIndex.getState();
  if (loading || (!force && Date.now() - at < FRESH_MS)) return;
  useSearchIndex.setState({ loading: true });
  const sessions = Object.values(useStore.getState().sessions).filter((s) => !s.lost);
  const lists = await Promise.all(sessions.map(forSession));
  useSearchIndex.setState({ items: lists.flat(), loading: false, at: Date.now() });
}

/** Opens a search result in its server's view, with that item selected. */
export function openItem(it: SearchItem) {
  const st = useStore.getState();
  if (!st.sessions[it.sid]) return;
  const id = it.key.slice(`${it.sid}:${it.kind}:`.length);
  useStore.setState({ globalView: null });
  st.setActive(it.sid);
  switch (it.kind) {
    case "service":
      moveTo(`${it.sid}:services:filter`, "all");
      moveTo(`${it.sid}:services:q`, "");
      moveTo(`${it.sid}:services:sel`, id);
      return st.setTab(it.sid, "services");
    case "container":
      moveTo(`${it.sid}:docker:q`, "");
      moveTo(`${it.sid}:docker:sel`, id);
      return st.openTool(it.sid, "docker");
    case "site":
      moveTo(`${it.sid}:nginx:q`, "");
      moveTo(`${it.sid}:nginx:sel`, id);
      return st.openTool(it.sid, "nginx");
    case "folder":
      return st.patch(it.sid, { tab: "files", cwd: id });
    case "file":
      st.patch(it.sid, { tab: "files" });
      return void st.openFile(it.sid, id);
  }
}
