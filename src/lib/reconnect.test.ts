import { beforeEach, describe, expect, it, vi } from "vitest";

// The backend: sessions in `alive` answer; any other session id is "not connected".
const alive = new Set<string>();
const calls: { cmd: string; args: any }[] = [];
vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {},
  invoke: vi.fn(async (cmd: string, args: any) => {
    calls.push({ cmd, args });
    const sid = args?.sessionId ?? args?.srcSessionId;
    if (sid && !alive.has(sid)) throw { kind: "NotConnected", message: "the connection to this server was lost", data: null };
    return `${cmd} on ${sid}`;
  }),
}));

const { api, setReconnector, setNotConnectedHandler, withSession, NO_RETRY } = await import("./api");

let reconnects = 0;
let lost: string[] = [];
beforeEach(() => {
  alive.clear();
  calls.length = 0;
  reconnects = 0;
  lost = [];
  setNotConnectedHandler((sid) => lost.push(sid));
});

describe("a dropped connection", () => {
  it("reconnects quietly and makes the call again on the new session", async () => {
    setReconnector(async () => { reconnects++; alive.add("s2"); return "s2"; });
    await expect(api.listDir("s1", "/etc")).resolves.toBe("list_dir on s2");
    expect(calls.map((c) => c.args.sessionId)).toEqual(["s1", "s2"]);
    expect(reconnects).toBe(1);
    expect(lost).toEqual([]);
    // Later calls with the old id go straight to the new session.
    await expect(api.readFile("s1", "/etc/hosts")).resolves.toBe("read_file on s2");
  });

  it("shares one reconnect between calls that fail together", async () => {
    setReconnector(async () => { reconnects++; await new Promise((r) => setTimeout(r, 20)); alive.add("t2"); return "t2"; });
    const r = await Promise.all([api.listDir("t1", "/"), api.listDir("t1", "/var"), api.readFile("t1", "/x")]);
    expect(r).toEqual(["list_dir on t2", "list_dir on t2", "read_file on t2"]);
    expect(reconnects).toBe(1);
  });

  it("falls back to Reconnect when it can't reconnect", async () => {
    setReconnector(async () => null);
    await expect(api.listDir("u1", "/")).rejects.toMatchObject({ kind: "NotConnected" });
    expect(lost).toEqual(["u1"]);
  });

  it("never repeats connecting, disconnecting or terminal input", async () => {
    setReconnector(async () => { reconnects++; return "v2"; });
    await expect(api.disconnect("v1")).rejects.toMatchObject({ kind: "NotConnected" });
    expect(reconnects).toBe(0);
    expect(NO_RETRY.test("terminal_write") && NO_RETRY.test("connect") && !NO_RETRY.test("list_dir")).toBe(true);
  });
});

it("withSession moves every session argument", () => {
  expect(withSession({ srcSessionId: "a", dstSessionId: "b", path: "/" }, "a", "c")).toEqual({ srcSessionId: "c", dstSessionId: "b", path: "/" });
});
