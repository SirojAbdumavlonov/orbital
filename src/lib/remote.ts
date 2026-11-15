import { useEffect, useRef, useState } from "react";
import { confirm } from "../components/ui";
import { asApiError } from "./api";

/** Load something from the server, with loading and error state and a reload. */
export function useRemote<T>(load: () => Promise<T>, deps: unknown[]) {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<{ kind: string; message: string } | null>(null);
  const [loading, setLoading] = useState(false);
  const seq = useRef(0);
  const reload = async () => {
    const n = ++seq.current;
    setLoading(true);
    try {
      const v = await load();
      if (n === seq.current) {
        setData(v);
        setError(null);
      }
    } catch (e) {
      const err = asApiError(e);
      if (n === seq.current) setError({ kind: err.kind, message: err.message });
    } finally {
      if (n === seq.current) setLoading(false);
    }
  };
  useEffect(() => { reload(); }, deps);
  return { data, setData, error, loading, reload };
}

/**
 * Run a change. If the engine refuses it because it would cut off this SSH
 * connection (or break a site), explain why and offer to do it anyway, which
 * requires typing a confirmation word. Resolves to the result, or null if the
 * user backed out.
 */
export async function withSafetyOverride<T>(run: (force: boolean) => Promise<T>, what: string): Promise<T | null> {
  try {
    return await run(false);
  } catch (e) {
    const err = asApiError(e);
    if (err.kind !== "PreflightFailed") throw e;
    const lockout = err.data?.check === "SSH access" || err.data?.check === "your access";
    const ok = await confirm({
      title: lockout ? "This could lock you out" : `Orbital stopped: ${what}`,
      body: err.data?.output ?? err.message,
      confirmLabel: "Do it anyway",
      danger: true,
      requireText: lockout ? "lock me out" : "anyway",
    });
    if (!ok) return null;
    return await run(true);
  }
}

/** 600 → "10 min", -1 → "forever". */
export function seconds(v: string | null | undefined): string {
  if (v == null || v === "") return "—";
  const n = Number(v);
  if (!Number.isFinite(n)) return v;
  if (n < 0) return "forever";
  if (n < 60) return `${n} s`;
  if (n < 3600) return `${Math.round(n / 60)} min`;
  if (n < 86400) return `${+(n / 3600).toFixed(1)} h`;
  return `${+(n / 86400).toFixed(1)} days`;
}
