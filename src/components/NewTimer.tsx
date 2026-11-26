import { useEffect, useState } from "react";
import { api, asApiError, ScheduleCheck, TimerSpec } from "../lib/api";
import { runJob } from "../lib/jobs";
import { SessionState } from "../store";
import { Modal } from "./ui";

const PRESETS: [string, string][] = [
  ["Every 15 minutes", "*:0/15"],
  ["Hourly", "hourly"],
  ["Daily at 03:00", "*-*-* 03:00:00"],
  ["Weekdays at 09:00", "Mon..Fri 09:00"],
  ["Weekly (Monday)", "weekly"],
  ["Monthly", "monthly"],
];

/**
 * A scheduled job as a systemd timer: the command runs as a one-off service,
 * its output lands in the journal, and systemd checks the schedule first.
 */
export function NewTimer({ s, onClose, onDone }: { s: SessionState; onClose: () => void; onDone: () => void }) {
  const sid = s.info.id;
  const [f, setF] = useState<TimerSpec>({ name: "", description: "", command: "", user: null, schedule: "*-*-* 03:00:00", persistent: true });
  const [check, setCheck] = useState<ScheduleCheck | null>(null);
  const [checkError, setCheckError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const set = <K extends keyof TimerSpec>(k: K, v: TimerSpec[K]) => setF((x) => ({ ...x, [k]: v }));

  useEffect(() => {
    if (!f.schedule.trim()) return;
    const t = setTimeout(() => {
      api.checkSchedule(sid, f.schedule).then((c) => { setCheck(c); setCheckError(null); }).catch((e) => { setCheck(null); setCheckError(asApiError(e).message); });
    }, 500);
    return () => clearTimeout(t);
  }, [f.schedule]);

  async function create() {
    setBusy(true);
    setError(null);
    try {
      await runJob((p) => api.createTimer(sid, f, p), { key: `${sid}:timer:${f.name}`, title: `New timer ${f.name}`, sid, done: () => `${f.name} scheduled` });
      onDone();
    } catch (e) {
      setError(asApiError(e).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal title="New scheduled job (systemd timer)" subtitle={`On ${s.profile.name}. Like cron, with its output in the journal and missed runs caught up.`} onClose={onClose} keepOnOutsideClick
      footer={<><button className="btn" onClick={onClose}>Cancel</button><button className="btn primary" disabled={busy || !f.name.trim() || !f.command.trim() || !!checkError} onClick={() => void create()}>{busy ? "Creating…" : "Create timer"}</button></>}>
      <div className="form">
        <div className="row-fields">
          <label className="field"><span>Name</span><input className="input mono" autoFocus placeholder="nightly-report" value={f.name} onChange={(e) => set("name", e.target.value.trim().toLowerCase())} /></label>
          <label className="field"><span>Run as</span><input className="input mono" placeholder="root" value={f.user ?? ""} onChange={(e) => set("user", e.target.value.trim() || null)} /></label>
        </div>
        <label className="field"><span>Command</span>
          <input className="input mono" placeholder="cd /srv/app && ./bin/cleanup --days 30" value={f.command} onChange={(e) => set("command", e.target.value)} />
          <small className="hint">Runs with <span className="mono">/bin/sh -c</span>. Its output goes to the journal: <span className="mono">journalctl -u {f.name || "NAME"}</span> (or the Logs tab).</small>
        </label>
        <label className="field"><span>Description (optional)</span><input className="input" value={f.description} onChange={(e) => set("description", e.target.value)} /></label>
        <div className="field"><span>When</span>
          <div className="chips">{PRESETS.map(([label, v]) => <button key={v} className={`chip ${f.schedule === v ? "on" : ""}`} onClick={() => set("schedule", v)}>{label}</button>)}</div>
          <input className="input mono" value={f.schedule} onChange={(e) => set("schedule", e.target.value)} aria-label="Schedule (systemd OnCalendar)" />
          <small className="hint">
            {checkError ? <span className="bad-text">{checkError}</span> : check ? <>systemd reads it as <span className="mono">{check.normalized}</span>; next run: <b>{check.next}</b> (server time).</> : "Checking with systemd…"}
          </small>
        </div>
        <label className="check"><input type="checkbox" checked={f.persistent} onChange={(e) => set("persistent", e.target.checked)} /><span>If the server was off at the time, run it once when it starts</span></label>
        {error && <div className="notice bad-text" style={{ margin: 0 }}>{error}</div>}
      </div>
    </Modal>
  );
}
