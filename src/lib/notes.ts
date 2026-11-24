import { create } from "zustand";

// Notes per server (Notes tab): a runbook, what runs where, who to call. Kept
// on this computer in localStorage `orbital.notes.<profile id>`, unencrypted,
// so the page says not to keep passwords there.

export interface Note {
  text: string;
  /** Unix ms of the last change. */
  updated: number;
}

const key = (profileId: string) => `orbital.notes.${profileId}`;

export function readNote(profileId: string): Note {
  try {
    const v = JSON.parse(localStorage.getItem(key(profileId)) ?? "null");
    if (v && typeof v.text === "string") return { text: v.text, updated: Number(v.updated) || 0 };
  } catch { /* none */ }
  return { text: "", updated: 0 };
}

/** The first non-empty line, without markdown heading marks, for previews. */
export function noteTitle(text: string): string {
  const line = text.split("\n").map((l) => l.trim()).find((l) => l.length) ?? "";
  return line.replace(/^#+\s*/, "").slice(0, 80);
}

export const useNotes = create<{ byProfile: Record<string, Note>; get(profileId: string): Note; set(profileId: string, text: string): void }>((set, get) => ({
  byProfile: {},
  get(profileId) {
    const have = get().byProfile[profileId];
    if (have) return have;
    const n = readNote(profileId);
    set((s) => ({ byProfile: { ...s.byProfile, [profileId]: n } }));
    return n;
  },
  set(profileId, text) {
    const n = { text, updated: Date.now() };
    set((s) => ({ byProfile: { ...s.byProfile, [profileId]: n } }));
    try {
      if (text.trim()) localStorage.setItem(key(profileId), JSON.stringify(n));
      else localStorage.removeItem(key(profileId));
    } catch { /* this run only */ }
  },
}));
