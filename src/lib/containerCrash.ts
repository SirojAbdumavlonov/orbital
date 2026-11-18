// "Why did this container stop?": Docker's own state for a container (from
// `docker inspect`) and its last log lines, explained in plain words. For the
// classic beginner question "my container keeps restarting".

export interface CrashFacts {
  status: string;
  exitCode: number | null;
  oomKilled: boolean;
  restarts: number;
  error: string | null;
  health: string | null;
  finishedAt: string | null;
  restartPolicy: string | null;
  memoryLimit: number | null;
}

/** The facts from `docker inspect` (State, RestartCount, HostConfig). */
export function crashFacts(inspect: any): CrashFacts | null {
  const st = inspect?.State;
  if (!st) return null;
  const fin = st.FinishedAt && !String(st.FinishedAt).startsWith("0001-") ? String(st.FinishedAt) : null;
  return {
    status: String(st.Status ?? (st.Restarting ? "restarting" : st.Running ? "running" : "exited")),
    exitCode: typeof st.ExitCode === "number" ? st.ExitCode : null,
    oomKilled: !!st.OOMKilled,
    restarts: Number(inspect.RestartCount ?? 0),
    error: st.Error ? String(st.Error) : null,
    health: st.Health?.Status ?? null,
    finishedAt: fin,
    restartPolicy: inspect.HostConfig?.RestartPolicy?.Name || null,
    memoryLimit: inspect.HostConfig?.Memory || null,
  };
}

/** Whether there is something to explain: it stopped badly, restarts, or is unhealthy. */
export function needsExplaining(f: CrashFacts): boolean {
  return f.status === "restarting" || f.oomKilled || f.health === "unhealthy" || !!f.error || (f.status !== "running" && f.exitCode !== null && f.exitCode !== 0) || (f.restarts > 0 && f.status !== "running");
}

export interface Explanation {
  level: "bad" | "warn";
  headline: string;
  /** What it means and what to try, in order. */
  hints: string[];
}

/** What exit code `code` usually means. */
export function exitMeaning(code: number): string {
  if (code === 0) return "it finished normally (exit code 0)";
  if (code === 1) return "the program reported an error and quit (exit code 1)";
  if (code === 2) return "the command was used wrongly, often a typo in the command or its options (exit code 2)";
  if (code === 125) return "Docker itself couldn't start it, for example a bad option (exit code 125)";
  if (code === 126) return "its command couldn't be run: not executable, or permission denied (exit code 126)";
  if (code === 127) return "its command wasn't found in the image (exit code 127)";
  if (code === 137) return "it was killed (SIGKILL, exit code 137): out of memory, or `docker stop` waited too long";
  if (code === 139) return "it crashed (segmentation fault, exit code 139)";
  if (code === 143) return "it was asked to stop (SIGTERM, exit code 143), usually `docker stop` or a restart";
  if (code > 128 && code < 160) return `it was ended by signal ${code - 128} (exit code ${code})`;
  return `the program quit with exit code ${code}`;
}

/** Explain a container's state, or null when it's fine. */
export function explain(f: CrashFacts, logTail: string): Explanation | null {
  if (!needsExplaining(f)) return null;
  const hints: string[] = [];
  let headline: string;
  if (f.oomKilled || (f.exitCode === 137 && f.memoryLimit)) {
    headline = "It ran out of memory and was killed";
    hints.push(f.memoryLimit ? `It has a memory limit of ${Math.round(f.memoryLimit / 1024 / 1024)} MB; raise it (mem_limit / --memory) or make the app use less.` : "The server itself ran out of memory: look at Monitor for what uses it, or add memory or swap.");
  } else if (f.exitCode !== null && f.exitCode !== 0 && f.status !== "running") {
    headline = `${f.status === "restarting" ? "It keeps restarting" : "It stopped"}: ${exitMeaning(f.exitCode)}`;
  } else if (f.health === "unhealthy") {
    headline = "It runs, but its health check fails";
    hints.push("The image's HEALTHCHECK command keeps failing: the app may be up but not answering. Inspect → State.Health shows the check's own output.");
  } else if (f.status === "restarting") {
    headline = "It keeps restarting";
  } else {
    headline = `It stopped (${f.status})`;
  }
  if (f.restarts > 0) hints.push(`Docker has restarted it ${f.restarts} ${f.restarts === 1 ? "time" : "times"}${f.restartPolicy ? ` (restart policy "${f.restartPolicy}")` : ""}; it will keep doing so until the cause is fixed.`);
  if (f.error) hints.push(`Docker said: ${f.error}`);
  if (f.exitCode === 127 || f.exitCode === 126) hints.push("Check the image's command or entrypoint (and the compose file's command:) for a wrong path or a missing file.");

  // The usual suspects in the last log lines.
  const log = logTail.toLowerCase();
  const seen = (re: RegExp) => re.test(log);
  if (seen(/address already in use|bind: address|port is already allocated/)) hints.push("The log says a port is already in use: another container or program has it. Open ports shows who.");
  if (seen(/permission denied|eacces/)) hints.push("The log mentions permission denied: a mounted folder's owner or mode may not match the user inside the container.");
  if (seen(/connection refused|econnrefused|could not connect|could not translate host name|name or service not known|getaddrinfo/)) hints.push("The log shows it can't reach something it needs (a database, another service). Is that one running, and is the host name right (the compose service name, not localhost)?");
  if (seen(/no such file or directory|enoent|cannot find module|modulenotfounderror/)) hints.push("The log says a file or module is missing: a volume path, a config file, or a dependency not in the image.");
  if (seen(/out of memory|cannot allocate memory|javascript heap out of memory|killed/)) hints.push("The log mentions memory: it may need more than it gets.");
  if (seen(/(missing|required|undefined).{0,40}(env|environment variable)|environment variable .{0,40}(not set|missing)|is not set/)) hints.push("The log mentions an environment variable that isn't set: check environment: / .env for the container.");
  if (seen(/syntax error|yaml|invalid configuration|config.*(error|invalid)/)) hints.push("The log points at a configuration error: the app's config file (often mounted into the container) has a mistake.");
  if (hints.length === 0 || !hints.some((h) => h.startsWith("The log"))) hints.push("Read the last log lines below: the cause is usually in the final few.");
  return { level: f.status === "running" && !f.oomKilled ? "warn" : "bad", headline, hints };
}

/** The last `n` non-empty lines of a log. */
export function lastLines(log: string, n = 15): string {
  return log.split("\n").filter((l) => l.trim()).slice(-n).join("\n");
}
