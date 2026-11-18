import { describe, expect, it } from "vitest";
import { crashFacts, exitMeaning, explain, lastLines, needsExplaining } from "./containerCrash";

const inspect = (state: object, extra: object = {}) => ({ State: { Status: "exited", ExitCode: 0, OOMKilled: false, Error: "", FinishedAt: "2026-11-18T10:00:00Z", ...state }, RestartCount: 0, HostConfig: { RestartPolicy: { Name: "unless-stopped" }, Memory: 0 }, ...extra });

describe("container crash explainer", () => {
  it("a healthy running container needs no explanation", () => {
    const f = crashFacts(inspect({ Status: "running", Health: { Status: "healthy" } }))!;
    expect(needsExplaining(f)).toBe(false);
    expect(explain(f, "")).toBeNull();
  });

  it("explains out of memory, with the limit", () => {
    const f = crashFacts(inspect({ Status: "restarting", ExitCode: 137, OOMKilled: true }, { RestartCount: 14, HostConfig: { RestartPolicy: { Name: "always" }, Memory: 268435456 } }))!;
    const e = explain(f, "starting worker\nKilled\n")!;
    expect(e.headline).toBe("It ran out of memory and was killed");
    expect(e.hints[0]).toContain("256 MB");
    expect(e.hints.some((h) => h.includes("restarted it 14 times"))).toBe(true);
    expect(e.level).toBe("bad");
  });

  it("reads the usual causes from the last log lines", () => {
    const f = crashFacts(inspect({ ExitCode: 1 }, { RestartCount: 3 }))!;
    const e = explain(f, "Error: connect ECONNREFUSED 127.0.0.1:5432\n    at TCPConnectWrap\n")!;
    expect(e.headline).toBe("It stopped: the program reported an error and quit (exit code 1)");
    expect(e.hints.some((h) => h.includes("can't reach something"))).toBe(true);
    const port = explain(f, "listen tcp 0.0.0.0:80: bind: address already in use")!;
    expect(port.hints.some((h) => h.includes("port is already in use"))).toBe(true);
    const env = explain(f, "FATAL: DATABASE_URL is not set")!;
    expect(env.hints.some((h) => h.includes("environment variable"))).toBe(true);
  });

  it("a restart loop says so", () => {
    const f = crashFacts(inspect({ Status: "restarting", Restarting: true, ExitCode: 1 }, { RestartCount: 14 }))!;
    expect(explain(f, "")!.headline).toBe("It keeps restarting: the program reported an error and quit (exit code 1)");
  });

  it("exit codes in words", () => {
    expect(exitMeaning(127)).toContain("wasn't found");
    expect(exitMeaning(143)).toContain("SIGTERM");
    expect(exitMeaning(130)).toContain("signal 2");
    expect(exitMeaning(3)).toBe("the program quit with exit code 3");
  });

  it("unhealthy, and a clean stop is fine", () => {
    expect(explain(crashFacts(inspect({ Status: "running", Health: { Status: "unhealthy" } }))!, "")!.headline).toBe("It runs, but its health check fails");
    expect(explain(crashFacts(inspect({ Status: "exited", ExitCode: 0 }))!, "")).toBeNull();
    expect(crashFacts(null)).toBeNull();
    expect(lastLines("a\n\nb\nc\n", 2)).toBe("b\nc");
  });
});
