import { useCallback, useEffect, useState } from "react";

/**
 * Where you were in a view (the selected container or service, search,
 * filters, sub-tab), kept while Orbital runs, so switching tabs and coming
 * back returns you to the same place. Keys say which server and view.
 */
const memory = new Map<string, unknown>();

/** The kept value for `key`, or `initial` if nothing was kept. */
export function keptValue<T>(key: string, initial: T): T {
  return memory.has(key) ? (memory.get(key) as T) : initial;
}

export function keepValue<T>(key: string, value: T) {
  memory.set(key, value);
}

/** Views showing a key now, told when something else sets it (search everywhere opening an item). */
const listeners = new Map<string, Set<(v: unknown) => void>>();

/** Set a kept value from outside its view; a view showing it now follows. */
export function moveTo<T>(key: string, value: T) {
  memory.set(key, value);
  for (const f of listeners.get(key) ?? []) f(value);
}

export function useKept<T>(key: string, initial: T): [T, (v: T | ((prev: T) => T)) => void] {
  const [value, setValue] = useState<T>(() => keptValue(key, initial));
  useEffect(() => {
    const f = (v: unknown) => setValue(v as T);
    const set = listeners.get(key) ?? new Set();
    set.add(f);
    listeners.set(key, set);
    return () => { set.delete(f); };
  }, [key]);
  const set = useCallback(
    (v: T | ((prev: T) => T)) =>
      setValue((prev) => {
        const next = typeof v === "function" ? (v as (p: T) => T)(prev) : v;
        keepValue(key, next);
        return next;
      }),
    [key],
  );
  return [value, set];
}

/** Take what was kept for one server (to give it to its new connection after a reconnect). */
export function takeKept(prefix: string): Map<string, unknown> {
  const out = new Map<string, unknown>();
  for (const [k, v] of memory) if (k.startsWith(prefix)) out.set(k.slice(prefix.length), v);
  return out;
}

export function restoreKept(prefix: string, kept: Map<string, unknown>) {
  for (const [k, v] of kept) memory.set(prefix + k, v);
}

/** Forget what was kept for a server (it disconnected). */
export function forgetKept(prefix: string) {
  for (const k of [...memory.keys()]) if (k.startsWith(prefix)) memory.delete(k);
}
