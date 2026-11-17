import { useEffect, useRef, useState } from "react";
import { CheckCircle2, ListChecks, Loader2, XCircle } from "lucide-react";
import { useJobs } from "../lib/jobs";
import { goTo, reachable } from "../store";
import { elapsed } from "./JobProgress";

/**
 * Sidebar item listing recent jobs on every server: what is running now (with
 * its current step) and what finished, with how long it took. Clicking one
 * goes back to where it was started. Shown once there has been a job.
 */
export function JobsMenu() {
  const jobs = useJobs((s) => s.jobs);
  const clear = useJobs((s) => s.clearFinished);
  const [open, setOpen] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  const running = jobs.filter((j) => j.status === "running").length;
  const [, tick] = useState(0);

  useEffect(() => {
    if (!open) return;
    const t = setInterval(() => tick((n) => n + 1), 1000);
    const away = (e: MouseEvent) => !box.current?.contains(e.target as Node) && setOpen(false);
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("mousedown", away);
    document.addEventListener("keydown", esc);
    return () => {
      clearInterval(t);
      document.removeEventListener("mousedown", away);
      document.removeEventListener("keydown", esc);
    };
  }, [open]);

  if (jobs.length === 0) return null;
  return (
    <div className="jobs-menu" ref={box}>
      <button className={`nav-item ${open ? "on" : ""}`} onClick={() => setOpen(!open)} aria-expanded={open}>
        {running ? <Loader2 size={15} className="spin amber" /> : <ListChecks size={15} />} Jobs
        {running > 0 && <span className="count">{running}</span>}
      </button>
      {open && (
        <div className="jobs-pop" role="dialog" aria-label="Jobs">
          <div className="jobs-pop-head">
            <b>Jobs</b>
            <span className="spacer" />
            {jobs.some((j) => j.status !== "running") && <button className="link" onClick={clear}>Clear finished</button>}
          </div>
          <ul>
            {jobs.map((j) => {
              const Icon = j.status === "running" ? Loader2 : j.status === "done" ? CheckCircle2 : XCircle;
              const canGo = reachable(j.loc);
              return (
                <li key={j.id}>
                  <button
                    className={`jobs-row ${j.status}`}
                    disabled={!canGo}
                    title={canGo ? "Go to where it was started" : "Its server is no longer connected"}
                    onClick={() => {
                      goTo(j.loc);
                      setOpen(false);
                    }}
                  >
                    <Icon size={14} className={j.status === "running" ? "spin" : undefined} />
                    <span className="jobs-text">
                      <span className="jobs-title">{j.title}</span>
                      <span className="jobs-sub faint">
                        {[j.server, j.status === "running" ? j.step ?? "Working…" : j.outcome].filter(Boolean).join(" · ")}
                      </span>
                    </span>
                    <span className="faint jobs-time">{elapsed((j.ended ?? Date.now()) - j.started)}</span>
                  </button>
                </li>
              );
            })}
          </ul>
        </div>
      )}
    </div>
  );
}
