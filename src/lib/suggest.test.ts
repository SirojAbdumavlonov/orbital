import { describe, expect, it } from "vitest";
import { LineTracker, nextSteps, suggest } from "./suggest";

describe("LineTracker", () => {
  it("follows typing, backspace and Enter", () => {
    const t = new LineTracker();
    expect(t.feed("systemctl stat")).toBeNull();
    t.feed("\x7f\x7f\x7f\x7f");
    expect(t.line).toBe("systemctl ");
    expect(t.feed("status nginx\r")).toBe("systemctl status nginx");
    expect(t.line).toBe("");
  });

  it("gives up on lines the shell edited itself", () => {
    const t = new LineTracker();
    t.feed("git sta\t");
    expect(t.known).toBe(false);
    expect(t.feed("\r")).toBeNull();
    t.feed("ls");
    t.feed("\x1b[D"); // ←
    expect(t.known).toBe(false);
    t.feed("\x03");
    expect(t.known).toBe(true);
    expect(t.line).toBe("");
    t.feed("\x1b[A\r"); // ↑ recalls history
    expect(t.feed("pwd\r")).toBe("pwd");
  });

  it("handles Ctrl+W, Ctrl+U and pasted text", () => {
    const t = new LineTracker();
    t.feed("docker compose logs");
    t.feed("\x17");
    expect(t.line).toBe("docker compose ");
    t.feed("\x15");
    expect(t.line).toBe("");
    expect(t.feed("\x1b[200~nginx -t\x1b[201~\r")).toBe("nginx -t");
  });
});

describe("nextSteps", () => {
  it("knows common follow-ups, keeping sudo", () => {
    expect(nextSteps("sudo nginx -t")).toEqual(["sudo systemctl reload nginx"]);
    expect(nextSteps("nginx -t")).toEqual(["systemctl reload nginx"]);
    expect(nextSteps("sudo systemctl restart php8.3-fpm")).toEqual(["sudo systemctl status php8.3-fpm", "sudo journalctl -u php8.3-fpm -n 50 --no-pager"]);
    expect(nextSteps("sudo vim /etc/nginx/sites-available/app")).toEqual(["sudo nginx -t"]);
    expect(nextSteps("docker compose pull")).toEqual(["docker compose up -d"]);
    expect(nextSteps("docker compose up -d --build")).toEqual(["docker compose ps", "docker compose logs -f --tail=100"]);
    expect(nextSteps("git clone git@github.com:me/shop.git")).toEqual(["cd shop"]);
    expect(nextSteps("git clone https://github.com/me/shop")).toEqual(["cd shop"]);
    expect(nextSteps("mkdir -p releases")).toEqual(["cd releases"]);
    expect(nextSteps("echo hi")).toEqual([]);
    expect(nextSteps(undefined)).toEqual([]);
  });
});

describe("suggest", () => {
  const history = ["docker compose logs -f", "systemctl status nginx", "sudo systemctl restart nginx", "ls -la"];

  it("an empty line gets the next steps after the last command", () => {
    expect(suggest("", { session: ["sudo nginx -t"], history })).toEqual(["sudo systemctl reload nginx"]);
    expect(suggest("", { session: [], history })).toEqual([]);
  });

  it("a started line gets matches: next steps, this session, then history", () => {
    expect(suggest("sys", { session: ["systemctl status ssh"], history })).toEqual(["systemctl restart ssh", "systemctl status ssh", "systemctl status nginx"]);
    expect(suggest("doc", { session: ["ls"], history })).toEqual(["docker compose logs -f"]);
    expect(suggest("sudo s", { session: ["sudo nginx -t"], history })).toEqual(["sudo systemctl reload nginx", "sudo systemctl restart nginx"]);
    expect(suggest("ls -la", { session: [], history })).toEqual([]);
  });

  it("at most four, each once, nothing for a line starting with a space", () => {
    const many = ["a1", "a2", "a3", "a4", "a5", "a1"];
    expect(suggest("a", { session: [], history: many })).toEqual(["a1", "a2", "a3", "a4"]);
    expect(suggest(" sys", { session: [], history })).toEqual([]);
  });
});
