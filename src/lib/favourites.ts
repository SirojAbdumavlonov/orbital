import { create } from "zustand";

// Favourite actions: one-click buttons under a server's name (restart nginx,
// open a config file, run a snippet…). Kept per server on this computer, in
// localStorage `orbital.favourites.<profile id>`. Pure helpers are tested.

export type FavKind = "restart" | "reload" | "container" | "file" | "folder" | "snippet";

export interface Favourite {
  id: string;
  kind: FavKind;
  /** A unit name, an absolute path, or a snippet id. */
  target: string;
  /** Shown on the button; a default is made from the target when empty. */
  label: string;
}

export const FAV_KINDS: { kind: FavKind; name: string }[] = [
  { kind: "restart", name: "Restart a service" },
  { kind: "reload", name: "Reload a service" },
  { kind: "container", name: "Restart a Docker container" },
  { kind: "file", name: "Open a file" },
  { kind: "folder", name: "Open a folder" },
  { kind: "snippet", name: "Run a snippet" },
];

const UNIT = /^[A-Za-z0-9@._:-]{1,200}$/;

/** Why a favourite can't be saved, or null. */
export function favProblem(kind: FavKind, target: string): string | null {
  const t = target.trim();
  if (!t) return kind === "snippet" ? "choose a snippet" : kind === "file" || kind === "folder" ? "give the path" : kind === "container" ? "choose the container" : "give the service's name";
  if ((kind === "restart" || kind === "reload") && !UNIT.test(t)) return "a service name has only letters, digits and @ . _ : -";
  if (kind === "container" && !/^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$/.test(t)) return "a container name has only letters, digits and _ . -";
  if ((kind === "file" || kind === "folder") && !t.startsWith("/")) return "the path must start with /";
  return null;
}

/** The button's text when no label was given. */
export function defaultLabel(kind: FavKind, target: string, snippetName?: string): string {
  const t = target.trim().replace(/\/+$/, "") || "/";
  const unit = t.replace(/\.service$/, "");
  switch (kind) {
    case "restart": return `Restart ${unit}`;
    case "reload": return `Reload ${unit}`;
    case "container": return `Restart ${t}`;
    case "file": return t.split("/").pop() || t;
    case "folder": return t === "/" ? "/" : `${t.split("/").pop()}/`;
    case "snippet": return snippetName ?? "Snippet";
  }
}

const key = (profileId: string) => `orbital.favourites.${profileId}`;

function load(profileId: string): Favourite[] {
  try {
    const v = JSON.parse(localStorage.getItem(key(profileId)) ?? "[]");
    return Array.isArray(v) ? v.filter((f) => f && FAV_KINDS.some((k) => k.kind === f.kind) && typeof f.target === "string") : [];
  } catch {
    return [];
  }
}

export const useFavourites = create<{
  byProfile: Record<string, Favourite[]>;
  list(profileId: string): Favourite[];
  add(profileId: string, f: Omit<Favourite, "id">): void;
  remove(profileId: string, id: string): void;
  move(profileId: string, id: string, by: -1 | 1): void;
}>((set, get) => {
  const save = (profileId: string, list: Favourite[]) => {
    try { localStorage.setItem(key(profileId), JSON.stringify(list)); } catch { /* this run only */ }
    set((s) => ({ byProfile: { ...s.byProfile, [profileId]: list } }));
  };
  return {
    byProfile: {},
    list(profileId) {
      const have = get().byProfile[profileId];
      if (have) return have;
      const loaded = load(profileId);
      set((s) => ({ byProfile: { ...s.byProfile, [profileId]: loaded } }));
      return loaded;
    },
    add(profileId, f) {
      const list = get().list(profileId);
      if (list.some((x) => x.kind === f.kind && x.target === f.target)) return;
      save(profileId, [...list, { ...f, id: `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}` }]);
    },
    remove(profileId, id) {
      save(profileId, get().list(profileId).filter((f) => f.id !== id));
    },
    move(profileId, id, by) {
      const list = [...get().list(profileId)];
      const i = list.findIndex((f) => f.id === id);
      const j = i + by;
      if (i < 0 || j < 0 || j >= list.length) return;
      [list[i], list[j]] = [list[j], list[i]];
      save(profileId, list);
    },
  };
});
