import { useEffect } from "react";
import { api, UptimeEvent } from "./api";
import { useStore } from "../store";

type Listener = (e: UptimeEvent) => void;
const listeners = new Set<Listener>();

/** Hear about uptime results while mounted (the Uptime page refreshes on them). */
export function useUptimeEvents(fn: Listener) {
  useEffect(() => {
    listeners.add(fn);
    return () => { listeners.delete(fn); };
  }, [fn]);
}

/**
 * Once, at the app's root: subscribe to the engine's uptime checks and toast
 * every change. The engine shows the macOS notification itself (so it shows
 * even while this window is hidden) and records the change in the activity log.
 */
export function useUptimeAlerts() {
  useEffect(() => {
    api.uptimeWatch((e) => {
      const { toast, setGlobalView } = useStore.getState();
      const open = { label: "Open Uptime", run: () => setGlobalView("uptime") };
      if (e.type === "changed") {
        if (e.change === "wentDown") toast({ kind: "error", title: `${e.name} is down`, body: e.error ?? e.target, action: open });
        else toast({ kind: "success", title: `${e.name} is back up`, body: e.target, action: open });
      } else if (e.type === "certWarning") {
        toast({ kind: "info", title: `${e.name}: certificate expiring`, body: e.message, action: open });
      }
      for (const l of listeners) l(e);
    }).catch(() => {});
  }, []);
}
