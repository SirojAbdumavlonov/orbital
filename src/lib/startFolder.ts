import { useEffect } from "react";
import type { Profile } from "./api";

// Which folder the Files tab opens in when a server connects (Settings →
// Files): the server's own start folder when its profile has one, else the
// user's choice: the home folder, / , the folder last open on that server
// (localStorage `orbital.lastFolder.<profile id>`), or a fixed path.
// `startFolder` is pure; tested.

export type FilesStart = "home" | "root" | "last" | "custom";

const key = (profileId: string) => `orbital.lastFolder.${profileId}`;

export function lastFolder(profileId: string): string | null {
  try { return localStorage.getItem(key(profileId)); } catch { return null; }
}

export function startFolder(home: string, profile: Pick<Profile, "initialPath">, mode: FilesStart, customPath: string, last: string | null): string {
  if (profile.initialPath?.startsWith("/")) return home; // the engine already used it
  switch (mode) {
    case "root": return "/";
    case "last": return last?.startsWith("/") ? last : home;
    case "custom": return customPath.trim().startsWith("/") ? customPath.trim().replace(/(.)\/+$/, "$1") : home;
    default: return home;
  }
}

/** Mounted once in App: remembers each server's Files folder, for "Last folder". */
export function useRememberFolders(subscribe: (f: (st: { sessions: Record<string, { profile: { id: string }; cwd: string; lost?: boolean }> }) => void) => () => void) {
  useEffect(() => {
    const seen: Record<string, string> = {};
    return subscribe((st) => {
      for (const s of Object.values(st.sessions)) {
        if (s.lost || !s.cwd || seen[s.profile.id] === s.cwd) continue;
        seen[s.profile.id] = s.cwd;
        try { localStorage.setItem(key(s.profile.id), s.cwd); } catch { /* this run only */ }
      }
    });
  }, []);
}
