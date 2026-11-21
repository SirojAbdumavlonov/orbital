import { useCallback, useState } from "react";

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

export function useKept<T>(key: string, initial: T): [T, (v: T | ((prev: T) => T)) => void] {
  const [value, setValue] = useState<T>(() => keptValue(key, initial));
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

/** Forget what was kept for a server (it disconnected). */
export function forgetKept(prefix: string) {
  for (const k of [...memory.keys()]) if (k.startsWith(prefix)) memory.delete(k);
}
