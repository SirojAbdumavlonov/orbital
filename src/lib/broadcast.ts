import { create } from "zustand";
import { api } from "./api";

// Typing into several terminals at once ("broadcast"): every open terminal,
// on any server, registers here; keys typed in one checked terminal are also
// sent to the other checked ones. Off by default, and each target shows a
// stripe while it's on. Pure parts (`fanOut`) are tested.

export interface LiveTerminal {
  /** `${sessionId}:${terminal key}` */
  id: string;
  tid: string;
  label: string;
}

export const useLiveTerminals = create<{ list: LiveTerminal[] }>(() => ({ list: [] }));

export function registerTerminal(t: LiveTerminal) {
  useLiveTerminals.setState((s) => ({ list: [...s.list.filter((x) => x.id !== t.id), t] }));
}

export function unregisterTerminal(id: string) {
  useLiveTerminals.setState((s) => ({ list: s.list.filter((x) => x.id !== id) }));
  const b = useBroadcast.getState();
  if (b.targets.includes(id)) b.setTargets(b.targets.filter((x) => x !== id));
}

export const useBroadcast = create<{ on: boolean; targets: string[]; setOn(on: boolean): void; setTargets(t: string[]): void; toggle(id: string): void }>((set, get) => ({
  on: false,
  targets: [],
  setOn: (on) => set({ on }),
  setTargets: (targets) => set({ targets, on: get().on && targets.length > 1 }),
  toggle(id) {
    const t = get().targets;
    get().setTargets(t.includes(id) ? t.filter((x) => x !== id) : [...t, id]);
  },
}));

/** Which terminal ids keys typed in `from` also go to: the other targets, when broadcasting from a target. */
export function fanOut(on: boolean, targets: string[], from: string): string[] {
  if (!on || !targets.includes(from)) return [];
  return targets.filter((t) => t !== from);
}

/** Sends what was typed in `from` to the other broadcast targets. */
export function broadcastFrom(from: string, data: string) {
  const { on, targets } = useBroadcast.getState();
  const live = useLiveTerminals.getState().list;
  for (const id of fanOut(on, targets, from)) {
    const t = live.find((x) => x.id === id);
    if (t) void api.terminalWrite(t.tid, data).catch(() => {});
  }
}
