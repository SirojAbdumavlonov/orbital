import { BarChart3 } from "lucide-react";
import { useState } from "react";
import { STATS_WHAT, useStats } from "../lib/stats";
import { Modal } from "./ui";

/** Asked once, after the first launch of a build that can send statistics. */
export function StatsPrompt() {
  const state = useStats((s) => s.state);
  const [busy, setBusy] = useState(false);
  if (!state?.available || state.enabled !== null) return null;
  const answer = async (on: boolean) => {
    setBusy(true);
    await useStats.getState().set(on).catch(() => {});
    setBusy(false);
  };
  return (
    <Modal
      title="Help make Orbital better?"
      keepOnOutsideClick
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
