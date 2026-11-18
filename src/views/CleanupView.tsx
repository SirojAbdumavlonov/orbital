import { HardDrive, RefreshCw, Trash2 } from "lucide-react";
import { useState } from "react";
import { JobProgress } from "../components/JobProgress";
import { confirm } from "../components/ui";
import { api, CleanupItem } from "../lib/api";
import { bytes } from "../lib/format";
import { runJob, useRunningJob } from "../lib/jobs";
import { useRemote } from "../lib/remote";
import { SessionState } from "../store";

/** Disk cleanup: where space can be freed safely, and freeing it in one click. */
export function CleanupView({ s }: { s: SessionState }) {
  const sid = s.info.id;
  const { data, error, loading, reload } = useRemote(() => api.cleanupScan(sid), [sid]);
  const [items, setItems] = useState<CleanupItem[] | null>(null);
  const list = items ?? data;
  const job = useRunningJob(`cleanup:${sid}`);
  const total = (list ?? []).reduce((n, i) => n + (i.bytes ?? 0), 0);
  const disks = s.metrics[s.metrics.length - 1]?.disks.filter((d) => d.mount === "/" || d.usedPercent >= 80) ?? [];

  async function free(item: CleanupItem) {
    const ok = await confirm({ title: `Free ${item.bytes ? bytes(item.bytes) : "space"}: ${item.title.toLowerCase()}?`, body: item.detail, confirmLabel: "Free space" });
    if (!ok) return;
    const before = item.bytes ?? 0;
    try {
      const after = await runJob((p) => api.cleanupRun(sid, item.id, p), {
        key: `cleanup:${sid}`,
        title: `Cleanup: ${item.title}`,
        sid,
        quietHere: false,
        done: (next) => {
          const left = next.find((x) => x.id === item.id)?.bytes ?? 0;
          return `Freed ${bytes(Math.max(0, before - left))}: ${item.title.toLowerCase()}`;
        },
      });
      setItems(after);
    } catch {
      // The job reports the error.
    }
  }

  return (
    <div className="view">
      <div className="toolbar">
        {list && <span className="faint">{bytes(total)} can be freed</span>}
        {disks.map((d) => <span key={d.mount} className={`badge ${d.usedPercent >= 90 ? "bad" : d.usedPercent >= 80 ? "amber" : ""}`}>{d.mount} {d.usedPercent.toFixed(0)}% full · {bytes(d.availableBytes)} free</span>)}
        <span className="spacer" />
        <button className="icon-btn" aria-label="Measure again" onClick={() => { setItems(null); reload(); }}><RefreshCw size={14} className={loading ? "spin" : ""} /></button>
      </div>
      {job && <div className="job-bar"><JobProgress job={job} /></div>}
      {error && <div className="empty"><div><h3>Couldn't measure</h3><p>{error.message}</p><button className="btn" onClick={reload}>Try again</button></div></div>}
      {!list && !error && <div className="empty"><div><RefreshCw size={22} className="spin faint" /><p>Measuring the usual places disks fill up…</p></div></div>}
      {list && (
        <div className="scroll">
          <ul className="checkup">
            {list.map((i) => (
              <li key={i.id} className="checkup-item ok">
                <HardDrive size={16} className="faint" />
                <div className="checkup-text">
                  <b>{i.title} <span className="faint" style={{ fontWeight: 400 }}>· {i.bytes === null ? "size unknown" : bytes(i.bytes)}</span></b>
                  <span className="faint">{i.detail}</span>
                </div>
                <button className="btn" disabled={!!job || i.bytes === 0} onClick={() => free(i)}><Trash2 size={13} /> Free</button>
              </li>
            ))}
          </ul>
          <p className="footnote" style={{ padding: "0 16px" }}>Only things that can go are listed: your files, databases, current logs and Docker volumes are never touched. Monitor → disk usage shows which folders use the rest.</p>
        </div>
      )}
    </div>
  );
}
