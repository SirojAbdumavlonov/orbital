import { describe, expect, it } from "vitest";
import { baseName, CopiedEntry, freeName, pasteAction } from "./paste";

const copied: CopiedEntry = { sid: "s1", path: "/etc/nginx/nginx.conf", name: "nginx.conf", dir: false, server: "web" };
const none = { files: [], text: null, image: null };

describe("pasteAction", () => {
  it("uploads files copied on this computer, even over Orbital's copy", () => {
    expect(pasteAction({ ...none, files: ["/Users/me/a.txt"] }, copied, { sid: "s1" })).toEqual({ kind: "upload", paths: ["/Users/me/a.txt"] });
  });
  it("pastes Orbital's copy while the clipboard still holds its path", () => {
    expect(pasteAction({ ...none, text: "/etc/nginx/nginx.conf" }, copied, { sid: "s1" }).kind).toBe("copySame");
    expect(pasteAction({ ...none, text: "/etc/nginx/nginx.conf\n" }, copied, { sid: "s2" }).kind).toBe("copyOther");
    expect(pasteAction(none, copied, { sid: "s1" }).kind).toBe("copySame");
  });
  it("ignores Orbital's copy once something else was copied", () => {
    expect(pasteAction({ ...none, text: "hello" }, copied, { sid: "s1" }).kind).toBe("nothing");
    expect(pasteAction({ ...none, text: "hello", image: "/tmp/p.png" }, copied, { sid: "s1" })).toEqual({ kind: "image", path: "/tmp/p.png" });
  });
  it("says what to do when there's nothing to paste", () => {
    expect(pasteAction(none, null, { sid: "s1" }).kind).toBe("nothing");
  });
});

describe("freeName", () => {
  it("keeps a free name and numbers copies like Finder", () => {
    expect(freeName("a.txt", ["b"])).toBe("a.txt");
    expect(freeName("a.txt", ["a.txt"])).toBe("a copy.txt");
    expect(freeName("a.txt", ["a.txt", "a copy.txt"])).toBe("a copy 2.txt");
    expect(freeName("app.tar.gz", ["app.tar.gz"])).toBe("app.tar copy.gz");
    expect(freeName(".env", [".env"])).toBe(".env copy");
    expect(freeName("v1.2", ["v1.2"], true)).toBe("v1.2 copy");
  });
});

it("baseName works for macOS, Linux and Windows paths", () => {
  expect(baseName("/Users/me/Desktop/report.pdf")).toBe("report.pdf");
  expect(baseName("C:\\Users\\me\\Desktop\\photos\\")).toBe("photos");
});
