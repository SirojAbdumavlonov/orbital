import { api } from "./api";

// Several Orbital windows: the main one, and ones opened for a server or a
// tab (`open_window` in commands.rs). They share the engine, so a window
// "attached" to a server uses the same connection. What should happen once
// for the whole app (alerts, update checks, saving the open servers for next
// launch) runs only in the main window. Pure parts are tested.

export interface WindowStart {
  /** An open session to show (its id). */
  attach: string | null;
  /** A saved server to connect to (its profile id). */
  connect: string | null;
  /** The tab to start on. */
  tab: string | null;
}

const ID = /^[A-Za-z0-9-]{1,64}$/;
const TAB = /^[A-Za-z0-9_:-]{1,64}$/;

/** What this window was opened for, from its address (`?attach=…&tab=…`). */
export function parseStart(search: string): WindowStart {
  const p = new URLSearchParams(search);
  const id = (k: string) => { const v = p.get(k); return v && ID.test(v) ? v : null; };
  const tab = p.get("tab");
  return { attach: id("attach"), connect: id("connect"), tab: tab && TAB.test(tab) ? tab : null };
}

export const start: WindowStart = typeof location === "undefined" ? { attach: null, connect: null, tab: null } : parseStart(location.search);

/** The first window: the one that runs alerts, update checks and session restore. */
export const isMainWindow = !start.attach && !start.connect && !(typeof location !== "undefined" && new URLSearchParams(location.search).has("window"));

/** Opens this server (and tab) in a window of its own, on the same connection. */
export function openServerWindow(sessionId: string, tab: string | null, title: string) {
  return api.openWindow({ attach: sessionId, tab, title: `${title} — Orbital` });
}

/** Opens a saved server in a new window, connecting there. */
export function openProfileWindow(profileId: string, title: string) {
  return api.openWindow({ connect: profileId, title: `${title} — Orbital` });
}
