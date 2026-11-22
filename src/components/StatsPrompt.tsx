import { BarChart3 } from "lucide-react";
import { useEffect, useState } from "react";
import { create } from "zustand";
import { STATS_WHAT, useStats } from "../lib/stats";
import { Modal } from "./ui";

/**
 * Remembered on this computer as soon as the question shows, so it is asked
 * once at all: not again after an update, a restart or closing it without an
 * answer. Settings → Privacy turns statistics on or off later.
 */
const ASKED_KEY = "orbital.statsAsked";
const askedBefore = () => {
  try { return localStorage.getItem(ASKED_KEY) === "1"; } catch { return false; }
};

/** Whether the question is on screen now (other launch dialogs wait for it). */
export const useStatsAsking = create<boolean>(() => false);

/** Asked once, after the first launch of a build that can send statistics. */
export function StatsPrompt() {
  const state = useStats((s) => s.state);
  const [busy, setBusy] = useState(false);
  // Decided once per run: a question already asked earlier never shows.
  const [skip] = useState(askedBefore);
  const [done, setDone] = useState(false);
  const show = !skip && !done && !!state?.available && state.enabled === null;
  useEffect(() => {
    useStatsAsking.setState(show, true);
    if (show) try { localStorage.setItem(ASKED_KEY, "1"); } catch { /* asked again next time */ }
  }, [show]);
  if (!show) return null;
  const answer = async (on: boolean) => {
    setBusy(true);
    await useStats.getState().set(on).catch(() => {});
    setBusy(false);
    setDone(true);
  };
  return (
    <Modal
      title="Help make Orbital better?"
      keepOnOutsideClick
      onClose={() => setDone(true)}
      footer={
        <>
          <button className="btn" disabled={busy} onClick={() => answer(false)}>No, thanks</button>
          <button className="btn primary" disabled={busy} onClick={() => answer(true)}>Share anonymous statistics</button>
        </>
      }
    >
      <div className="stats-ask">
        <BarChart3 size={22} />
        <div>
          <p>Orbital can count how many people use it and for how long, so we know what to improve. Nothing about your servers leaves this computer.</p>
          <p className="faint">{STATS_WHAT}</p>
          <p className="faint">You can change this any time in Settings → Privacy.</p>
        </div>
      </div>
    </Modal>
  );
}
