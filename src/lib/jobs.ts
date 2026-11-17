import { create } from "zustand";
import { api, asApiError, type OnProgress, type ProgressEvent } from "./api";
import { currentLoc, goTo, reachable, sameLoc, useStore, type Loc } from "../store";

// Long operations ("jobs"): renewing a certificate, copying between servers,
// `docker compose pull`. Each one shows what is happening while it runs (the
// engine's steps and the command's own output, see orbital_core::progress),
// keeps running when the user moves to another tab, and says when it's done:
// a toast with "Show" when the user is elsewhere, a system notification when
// Orbital is in the background.

export interface Job {
  id: string;
  /** Finds the job again after the view that started it was left and reopened (e.g. `certbot:<sid>`). */
  key: string;
  title: string;
  /** Server name, for the notification. */
  server?: string;
  /** Where it was started: "Show" goes back there. */
  loc: Loc;
  status: "running" | "done" | "failed";
  /** The latest step in words, and the latest line a command printed. */
  step?: string;
  last?: string;
  lines: string[];
  transfer?: { files: number; bytes: number; current: string };
  started: number;
  ended?: number;
  /** How it ended, in words. */
  outcome?: string;
}

const MAX_LINES = 300;
const KEEP = 40;

interface JobsState {
  jobs: Job[];
  update(id: string, f: (j: Job) => Partial<Job>): void;
  clearFinished(): void;
}

export const useJobs = create<JobsState>((set) => ({
  jobs: [],
  update(id, f) {
    set((s) => ({ jobs: s.jobs.map((j) => (j.id === id ? { ...j, ...f(j) } : j)) }));
  },
  clearFinished() {
    set((s) => ({ jobs: s.jobs.filter((j) => j.status === "running") }));
  },
}));

/** Apply one progress event to a job (pure, for tests). */
export function applyProgress(j: Job, e: ProgressEvent): Partial<Job> {
  switch (e.type) {
    case "step":
      return { step: e.text, lines: [...j.lines, `› ${e.text}`].slice(-MAX_LINES) };
    case "line":
      return { last: e.text, lines: [...j.lines, e.text].slice(-MAX_LINES) };
    case "transfer":
      return { transfer: { files: e.files, bytes: e.bytes, current: e.current } };
  }
}

export interface JobOptions<T> {
  key: string;
  title: string;
  /** The server it runs on, for the notification and "Show". */
  sid?: string;
  /** How it went, in words ("example.com renewed"), or `{ failed }` when the result means it didn't work. */
  done?: (r: T) => string | { failed: string };
  /**
   * While the user is still looking at it, the view shows the result itself
   * (an output panel), so the job says nothing. `false`: a toast either way.
   * Errors are always reported, and away from the view always come with "Show".
   */
  quietHere?: boolean;
}

let seq = 0;

/** Run a long operation as a job (`run` first, so its result types `opts.done`). Resolves or rejects like `run`. */
export async function runJob<T>(run: (onProgress: OnProgress) => Promise<T>, opts: JobOptions<T>): Promise<T> {
  const st = useStore.getState();
  const id = `job-${Date.now()}-${++seq}`;
  const job: Job = {
    id,
    key: opts.key,
    title: opts.title,
    server: opts.sid ? st.sessions[opts.sid]?.profile.name : undefined,
    loc: currentLoc(),
    status: "running",
    lines: [],
    started: Date.now(),
  };
  useJobs.setState((s) => ({ jobs: [job, ...s.jobs].slice(0, KEEP) }));
  const { update } = useJobs.getState();
  try {
    const r = await run((e) => update(id, (j) => applyProgress(j, e)));
    const d = opts.done?.(r);
    const failed = typeof d === "object" ? d.failed : undefined;
    update(id, () => ({ status: failed ? "failed" : "done", ended: Date.now(), outcome: failed ?? (typeof d === "string" ? d : "Done") }));
    finished(id, opts.quietHere !== false);
    return r;
  } catch (e) {
    const err = asApiError(e);
    update(id, () => ({ status: "failed", ended: Date.now(), outcome: err.message }));
    // A failed pre-flight check is explained by the view's own dialog (withSafetyOverride).
    finished(id, err.kind === "PreflightFailed");
    throw e;
  }
}

const isHere = (j: Job) => sameLoc(currentLoc(), j.loc) && document.visibilityState === "visible" && document.hasFocus();

/** Tell the user how a job ended: a toast (with "Show" when they're elsewhere), and the system when Orbital is in the background. */
function finished(id: string, quietHere: boolean) {
  const j = useJobs.getState().jobs.find((x) => x.id === id);
  if (!j) return;
  const here = isHere(j);
  const ok = j.status === "done";
  const title = ok ? j.outcome && j.outcome !== "Done" ? j.outcome : `${j.title}: done` : `${j.title}: failed`;
  const body = [j.server, ok ? undefined : j.outcome].filter(Boolean).join(" · ") || undefined;
  if (!(here && quietHere)) {
    const away = !sameLoc(currentLoc(), j.loc) && reachable(j.loc);
    useStore.getState().toast({ kind: ok ? "success" : "error", title, body, action: away ? { label: "Show", run: () => goTo(j.loc) } : undefined });
  }
  if (document.visibilityState !== "visible" || !document.hasFocus()) {
    void api.notify(title, body ?? "").catch(() => {});
  }
}

/** The newest job with this key: the one a view started, still there after it was reopened. */
export function useJob(key: string): Job | undefined {
  return useJobs((s) => s.jobs.find((j) => j.key === key));
}

/** The newest job with this key while it runs. */
export function useRunningJob(key: string): Job | undefined {
  return useJobs((s) => s.jobs.find((j) => j.key === key && j.status === "running"));
}
