import { useEffect } from "react";
import { SavedPlace, useStore } from "../store";
import { useSettings } from "./settings";

// Reopen the servers that were open when Orbital closed (Settings → Starting
// Orbital). Where you were in each (tab, folder, tool tabs) and which one was
// in front are kept on this computer; nothing secret is stored.

const KEY = "orbital.lastSession";

interface Saved {
  places: SavedPlace[];
  active: string | null;
}

/** What to remember: every connected server (in sidebar order of opening), and the one in front. */
export function snapshot(): Saved {
  const st = useStore.getState();
  const places = st.order
    .map((id) => st.sessions[id])
    .filter((s) => s && !s.lost)
    .map((s) => ({ profileId: s.profile.id, tab: s.tab, cwd: s.cwd, toolTabs: s.toolTabs }));
  const active = st.active ? st.sessions[st.active]?.profile.id ?? null : null;
  return { places, active };
}

export function readSaved(): Saved | null {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "null");
    if (!v || !Array.isArray(v.places)) return null;
    const places = v.places.filter((p: any) => p && typeof p.profileId === "string" && typeof p.tab === "string");
    return { places, active: typeof v.active === "string" ? v.active : null };
  } catch {
    return null;
  }
}

/**
 * Mounted once in App, after the server list has loaded: reopens the saved
 * servers (when the setting is on), then keeps the record up to date.
 * Saving starts only after the restore, so an empty start never wipes it.
 */
export function useSessionRestore(profilesLoaded: boolean) {
  useEffect(() => {
    if (!profilesLoaded) return;
    let unsub = () => {};
    let timer: number | undefined;
    const save = () => {
      clearTimeout(timer);
      timer = window.setTimeout(() => {
        try { localStorage.setItem(KEY, JSON.stringify(snapshot())); } catch { /* not remembered */ }
      }, 500);
    };
    void (async () => {
      const saved = readSaved();
      if (saved && saved.places.length && useSettings.getState().restoreSession) {
        const skipped = await useStore.getState().restoreSessions(saved.places, saved.active);
        if (skipped.length) {
          useStore.getState().toast({
            kind: "info",
            title: skipped.length === 1 ? `${skipped[0]} wasn't reopened` : `${skipped.length} servers weren't reopened`,
            body: `${skipped.join(", ")}: it needs you (a password or a new host key) or didn't answer. Click it in the sidebar to connect.`,
          });
        }
      }
      unsub = useStore.subscribe((st, prev) => {
        if (st.sessions !== prev.sessions || st.active !== prev.active || st.order !== prev.order) save();
      });
    })();
    return () => {
      unsub();
      clearTimeout(timer);
    };
  }, [profilesLoaded]);
}
