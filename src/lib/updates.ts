import { useEffect } from "react";
import { create } from "zustand";
import { useSettings } from "./settings";
import { useStore } from "../store";

// In-app updates (tauri-plugin-updater). Each release publishes latest.json
// next to a signed Orbital.app.tar.gz; the app installs an update only if its
// signature matches the public key in tauri.conf.json. Nothing is installed
// without the user clicking Update, and the new version runs after a restart.

export type UpdateStatus = "idle" | "checking" | "upToDate" | "available" | "downloading" | "ready" | "error";

interface UpdateState {
  status: UpdateStatus;
  /** The version on offer, and its release notes. */
  version?: string;
  notes?: string;
  date?: string;
  /** Bytes downloaded so far, and the size when the server sent it. */
  got: number;
  total?: number;
  error?: string;
  checkedAt?: number;
  /** `quiet`: a background check, which says nothing unless there is an update. */
  check(quiet?: boolean): Promise<void>;
  install(): Promise<void>;
  restart(): Promise<void>;
}

type Update = Awaited<ReturnType<typeof import("@tauri-apps/plugin-updater").check>>;
let pending: Update = null;
/** The version already announced in a toast, so background checks don't repeat it. */
let announced: string | undefined;

/**
 * `npm run tauri dev` builds aren't installed copies: there is nothing to
 * update, and asking would only log the updater's error. The demo (a fake
 * backend) still shows the whole flow.
 */
export const updatesDisabled = import.meta.env.DEV && !import.meta.env.VITE_DEMO;

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

export const useUpdates = create<UpdateState>((set, get) => ({
  status: "idle",
  got: 0,

  async check(quiet = false) {
    const busy = ["checking", "downloading", "ready"].includes(get().status);
    if (busy) return;
    if (updatesDisabled) {
      if (!quiet) set({ status: "error", error: "development builds don't update themselves; this works in the installed app", checkedAt: Date.now() });
      return;
    }
    set({ status: "checking", error: undefined });
    try {
      const { check } = await import("@tauri-apps/plugin-updater");
      const u = await check({ timeout: 20000 });
      await pending?.close().catch(() => {});
      pending = u;
      set({ checkedAt: Date.now() });
      if (!u) {
        set({ status: "upToDate", version: undefined, notes: undefined });
        return;
      }
      set({ status: "available", version: u.version, notes: u.body?.trim() || undefined, date: u.date });
      // A check from Settings shows the result there; a background one announces it once.
      if (quiet && announced !== u.version) {
        announced = u.version;
        useStore.getState().toast({
          kind: "info",
          title: `Orbital ${u.version} is available`,
          body: "Download it now; it is used the next time Orbital starts.",
          action: { label: "Update", run: () => void get().install() },
        });
      }
    } catch (e) {
      // Offline, or no release published yet: a background check stays silent.
      set({ status: quiet ? "idle" : "error", error: message(e), checkedAt: Date.now() });
    }
  },

  async install() {
    const u = pending;
    if (!u || get().status === "downloading" || get().status === "ready") return;
    set({ status: "downloading", got: 0, total: undefined, error: undefined });
    try {
      await u.downloadAndInstall((ev) => {
        if (ev.event === "Started") set({ total: ev.data.contentLength });
        else if (ev.event === "Progress") set((s) => ({ got: s.got + ev.data.chunkLength }));
      });
      set({ status: "ready" });
      useStore.getState().toast({
        kind: "success",
        title: `Orbital ${u.version} is installed`,
        body: "Restart to start using it. Restarting closes open connections and terminals.",
        action: { label: "Restart now", run: () => void get().restart() },
      });
    } catch (e) {
      set({ status: "error", error: message(e) });
      useStore.getState().toast({ kind: "error", title: "The update couldn't be installed", body: message(e) });
    }
  },

  async restart() {
    const { relaunch } = await import("@tauri-apps/plugin-process");
    await relaunch();
  },
}));

const FIRST_CHECK_MS = 8_000;
const EVERY_MS = 6 * 60 * 60 * 1000;

/** Checks for updates shortly after launch and every six hours, when the setting is on. */
export function useUpdateChecks() {
  const auto = useSettings((s) => s.autoUpdates);
  useEffect(() => {
    // Only inside the app (or the demo's mock backend), never in a plain browser.
    if (!auto || updatesDisabled || !("__TAURI_INTERNALS__" in window)) return;
    const quiet = () => void useUpdates.getState().check(true);
    const first = setTimeout(quiet, FIRST_CHECK_MS);
    const every = setInterval(quiet, EVERY_MS);
    return () => {
      clearTimeout(first);
      clearInterval(every);
    };
  }, [auto]);
}
