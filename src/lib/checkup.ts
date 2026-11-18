import type { CertbotStatus, Container, Metrics, SshdReport, SystemInfo, UfwStatus } from "./api";
import { bytes } from "./format";

// Server checkup: what needs attention on a server, in plain words, each with
// the tab that fixes it. Built only from read-only views Orbital already has.

export type Level = "bad" | "warn" | "ok";

export interface Finding {
  level: Level;
  title: string;
  detail: string;
  /** The tab that deals with it. */
  go?: { label: string; tab: string };
}

export interface CheckupInput {
  info: SystemInfo | null;
  metrics: Metrics | null;
  sshd: SshdReport | null;
  /** null: couldn't read it; "absent": ufw isn't installed. */
  ufw: UfwStatus | "absent" | null;
  /** null: couldn't read it or certbot isn't installed. */
  certs: CertbotStatus | null;
  /** Docker containers; null without Docker. */
  containers?: Container[] | null;
}

const ORDER: Record<Level, number> = { bad: 0, warn: 1, ok: 2 };

export function checkup(x: CheckupInput): Finding[] {
  const out: Finding[] = [];
  const add = (f: Finding) => out.push(f);
  const monitor = { label: "Open Monitor", tab: "monitor" };

  // Disks
  if (x.metrics) {
    const full = x.metrics.disks.filter((d) => d.totalBytes > 0 && d.usedPercent >= 80);
    for (const d of full) {
      add({
        level: d.usedPercent >= 90 ? "bad" : "warn",
        title: `Disk ${d.mount} is ${d.usedPercent.toFixed(0)}% full`,
        detail: `${bytes(d.availableBytes)} left of ${bytes(d.totalBytes)}. When it fills up, databases and logs stop writing and services fail. Disk cleanup frees the usual leftovers; Monitor shows which folders use the rest.`,
        go: { label: "Free up space", tab: "tool:cleanup" },
      });
    }
    if (!full.length && x.metrics.disks.length) add({ level: "ok", title: "Disks have room", detail: `Every disk is under 80% full.` });

    // Memory
    const m = x.metrics;
    if (m.memTotal > 0) {
      const avail = m.memAvailable / m.memTotal;
      if (avail < 0.2) {
        add({
          level: avail < 0.1 ? "bad" : "warn",
          title: `Only ${bytes(m.memAvailable)} of memory is available`,
          detail: `${(avail * 100).toFixed(0)}% of ${bytes(m.memTotal)}. When it runs out the server slows down${m.swapTotal ? " and swaps" : ""}, and Linux stops the biggest program. Monitor shows what uses it.`,
          go: monitor,
        });
      } else add({ level: "ok", title: "Enough free memory", detail: `${bytes(m.memAvailable)} of ${bytes(m.memTotal)} available.` });
    }
    if (m.load[0] > m.cores * 1.5) {
      add({ level: "warn", title: "The CPU is overloaded", detail: `Load ${m.load[0].toFixed(2)} on ${m.cores} cores: more work is waiting than the CPU can do. Monitor shows which programs use it.`, go: monitor });
    }
  }

  // Services, updates, reboot
  if (x.info) {
    const failed = x.info.failedUnits;
    if (failed.length) {
      add({ level: "bad", title: `${failed.length} ${failed.length === 1 ? "service has" : "services have"} failed`, detail: `${failed.slice(0, 5).join(", ")}${failed.length > 5 ? ", …" : ""}. Open Services to see their logs and start them again.`, go: { label: "Open Services", tab: "services" } });
    } else add({ level: "ok", title: "No failed services", detail: "Every systemd service that should run is running." });
    const sec = x.info.securityUpdates ?? 0;
    const all = x.info.updates ?? 0;
    const updates = { label: "Open Updates", tab: "tool:updates" };
    if (sec > 0) add({ level: "bad", title: `${sec} security ${sec === 1 ? "update" : "updates"} waiting`, detail: `Of ${all} updates in all. Security updates fix holes attackers know about; install them soon.`, go: updates });
    else if (all > 0) add({ level: "warn", title: `${all} ${all === 1 ? "update" : "updates"} available`, detail: "None of them is a security update.", go: updates });
    else if (x.info.updates !== null) add({ level: "ok", title: "Up to date", detail: "No package updates waiting." });
    if (x.info.rebootRequired) add({ level: "warn", title: "A reboot is needed", detail: "Updates (often a new kernel) only take effect after the server restarts.", go: updates });
  }

  // Firewall
  if (x.ufw === "absent") add({ level: "warn", title: "No firewall found", detail: "ufw isn't installed, so every port a program opens is reachable from the internet (unless your provider filters it). Installing ufw: apt install ufw.", go: { label: "Open ports", tab: "tool:ports" } });
  else if (x.ufw && !x.ufw.active) add({ level: "warn", title: "The firewall is off", detail: "ufw is installed but not on. Turn it on after allowing SSH; Orbital checks you won't lock yourself out.", go: { label: "Open Firewall", tab: "tool:ufw" } });
  else if (x.ufw) add({ level: "ok", title: "Firewall is on", detail: `ufw with ${x.ufw.rules.length} ${x.ufw.rules.length === 1 ? "rule" : "rules"}, incoming ${x.ufw.defaultIncoming ?? "?"} by default.` });

  // SSH
  if (x.sshd?.readable) {
    const bad = x.sshd.checks.filter((c) => c.level === "bad" || c.level === "warn");
    for (const c of bad) add({ level: c.level === "bad" ? "bad" : "warn", title: `SSH: ${c.title}`, detail: c.advice, go: { label: "Open SSH server", tab: "tool:sshd" } });
    if (!bad.length) add({ level: "ok", title: "SSH is set up safely", detail: "No risky sshd settings found." });
  }

  // Certificates
  if (x.certs && !x.certs.error) {
    let worry = false;
    for (const c of x.certs.certificates) {
      const d = c.daysLeft;
      if (!c.valid || (d !== null && d < 7)) {
        worry = true;
        add({ level: "bad", title: `Certificate ${c.name} ${!c.valid || (d ?? 0) <= 0 ? "has expired" : `expires in ${d} days`}`, detail: "Browsers will show a security warning for this site. Renew it, and check why automatic renewal didn't.", go: { label: "Open certificates", tab: "tool:certbot" } });
      } else if (d !== null && d < 20) {
        worry = true;
        add({ level: "warn", title: `Certificate ${c.name} expires in ${d} days`, detail: "Certbot normally renews 30 days ahead, so automatic renewal may be failing. Try a renewal test.", go: { label: "Open certificates", tab: "tool:certbot" } });
      }
    }
    if (x.certs.autoRenew && !x.certs.autoRenew.active && x.certs.certificates.length) {
      worry = true;
      add({ level: "warn", title: "Certificates don't renew automatically", detail: `${x.certs.autoRenew.via} is off, so certificates will expire in 90 days.`, go: { label: "Open certificates", tab: "tool:certbot" } });
    }
    if (!worry && x.certs.certificates.length) add({ level: "ok", title: "Certificates are fine", detail: `${x.certs.certificates.length} valid, renewing automatically.` });
  }

  // Containers that keep restarting or stopped with an error
  if (x.containers) {
    const docker = { label: "Open Docker", tab: "tool:docker" };
    const code = (c: Container) => Number(/\((\d+)\)/.exec(c.status)?.[1] ?? NaN);
    const looping = x.containers.filter((c) => c.state === "restarting");
    const crashed = x.containers.filter((c) => c.state === "exited" && code(c) !== 0 && code(c) !== 143 && !Number.isNaN(code(c)));
    for (const c of looping) add({ level: "bad", title: `Container ${c.names} keeps restarting`, detail: "It starts, fails and starts again. Docker → the container shows why it stops and its last log lines.", go: docker });
    for (const c of crashed) add({ level: "warn", title: `Container ${c.names} stopped with an error`, detail: `${c.status}. Docker → the container explains the exit code and shows its last log lines.`, go: docker });
    if (!looping.length && !crashed.length && x.containers.length) add({ level: "ok", title: "Containers are fine", detail: `${x.containers.filter((c) => c.state === "running").length} running, none restarting or crashed.` });
  }

  return out.sort((a, b) => ORDER[a.level] - ORDER[b.level]);
}
