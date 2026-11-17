import { useEffect, useState } from "react";
import { CheckCircle2, ChevronDown, ChevronRight, CircleSlash, Loader2, Square, XCircle } from "lucide-react";
import { stopJob, type Job } from "../lib/jobs";
import { bytes } from "../lib/format";

/** "12 s", "1 min 5 s". */
export function elapsed(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  return s < 60 ? `${s} s` : `${Math.floor(s / 60)} min ${s % 60} s`;
}

/** Ticks once a second while `on`, so elapsed times stay current. */
function useTick(on: boolean) {
  const [, set] = useState(0);
  useEffect(() => {
    if (!on) return;
    const t = setInterval(() => set((n) => n + 1), 1000);
    return () => clearInterval(t);
  }, [on]);
}

/**
 * What a long operation is doing, live: the current step, the latest line
 * its command printed, files and bytes for transfers, and the time so far.
 * "Details" shows everything it printed.
 */
export function JobProgress({ job, compact }: { job: Job | undefined; compact?: boolean }) {
  const running = job?.status === "running";
  useTick(running);
  const [open, setOpen] = useState(false);
  if (!job) return null;
  const took = elapsed((job.ended ?? Date.now()) - job.started);
  const Icon = running ? Loader2 : job.status === "done" ? CheckCircle2 : job.status === "stopped" ? CircleSlash : XCircle;
  const t = job.transfer;
  return (
    <div className={`job ${job.status}${compact ? " compact" : ""}`} role="status" aria-live="polite">
      <div className="job-head">
        <Icon size={14} className={running ? "spin" : undefined} />
        <span className="job-step">{running ? job.step ?? `${job.title} …` : job.outcome ?? job.title}</span>
        <span className="job-time faint">{took}</span>
        {running && job.jobId !== undefined && (
          <button className="link job-stop" onClick={() => void stopJob(job)} title="Stop it: the command on the server is ended">
            <Square size={10} /> Stop
          </button>
        )}
        {job.lines.length > 0 && (
          <button className="link job-toggle" onClick={() => setOpen(!open)} aria-expanded={open}>
            {open ? <ChevronDown size={12} /> : <ChevronRight size={12} />} Details
          </button>
        )}
      </div>
      {running && t && (
        <div className="job-sub mono faint">
          {t.files} {t.files === 1 ? "file" : "files"} · {bytes(t.bytes)} · {t.current.split("/").pop()}
        </div>
      )}
      {running && !t && job.last && <div className="job-sub mono faint" title={job.last}>{job.last}</div>}
      {open && <pre className="job-log mono">{job.lines.join("\n")}</pre>}
    </div>
  );
}
