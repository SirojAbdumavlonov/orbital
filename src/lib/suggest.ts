// Terminal command suggestions: what the user is probably about to type,
// from the server's shell history, the commands typed in this session, and
// common next steps (after `nginx -t`, `systemctl reload nginx`). Suggestions
// are only ever typed into the prompt when the user accepts one; Enter is
// always theirs to press.

/**
 * Follows what is typed at the prompt, from the keys sent to the shell.
 * It can't see the shell's own line editing (Tab completion, history with ↑,
 * moving the cursor), so after any of those the line is `known: false` until
 * the next Enter, and no suggestion is shown.
 */
export class LineTracker {
  line = "";
  known = true;

  /** Feed keys sent to the terminal. Returns the line when Enter was pressed on a known line. */
  feed(data: string): string | null {
    let committed: string | null = null;
    // Bracketed paste markers wrap pasted text; the text itself counts as typed.
    data = data.replace(/\x1b\[20[01]~/g, "");
    for (let i = 0; i < data.length; i++) {
      const c = data[i];
      const code = c.charCodeAt(0);
      if (c === "\r" || c === "\n") {
        if (this.known && this.line.trim()) committed = this.line;
        this.reset();
      } else if (c === "\x7f" || c === "\b") {
        this.line = this.line.slice(0, -1);
      } else if (c === "\x03" || c === "\x15") {
        // Ctrl+C, Ctrl+U: the line is gone.
        this.reset();
      } else if (c === "\x17") {
        // Ctrl+W: the word before the cursor.
        this.line = this.line.replace(/\S+\s*$/, "");
      } else if (c === "\x1b") {
        // An escape sequence (arrows, Home/End, Alt+key): the shell moved or
        // replaced something we can't follow. Skip the sequence.
        this.known = false;
        if (data[i + 1] === "[" || data[i + 1] === "O") {
          i += 2;
          while (i < data.length && !/[@-~]/.test(data[i])) i++;
        } else i++;
      } else if (code < 0x20) {
        // Tab completion and other control keys change the line in the shell.
        this.known = false;
      } else {
        this.line += c;
      }
    }
    return committed;
  }

  reset() {
    this.line = "";
    this.known = true;
  }
}

interface Rule {
  re: RegExp;
  next: (m: RegExpMatchArray) => string[];
}

const S = (m: RegExpMatchArray) => (m[1] ? "sudo " : "");

/** Common next steps after a command. `(sudo )?` at the start carries over to commands that need root. */
const RULES: Rule[] = [
  { re: /^(sudo )?nginx -t$/, next: (m) => [`${S(m)}systemctl reload nginx`] },
  { re: /^(sudo )?(?:nano|vim?|micro) (\/etc\/nginx\/\S+)$/, next: (m) => [`${S(m)}nginx -t`] },
  { re: /^(sudo )?(?:nano|vim?|micro) (\/etc\/ssh\/sshd_config\S*)$/, next: (m) => [`${S(m)}sshd -t`] },
  { re: /^(sudo )?sshd -t$/, next: (m) => [`${S(m)}systemctl reload ssh`] },
  { re: /^(sudo )?apache2ctl configtest$/, next: (m) => [`${S(m)}systemctl reload apache2`] },
  { re: /^(sudo )?systemctl (?:start|restart|reload) (\S+)$/, next: (m) => [`${S(m)}systemctl status ${m[2]}`, `${S(m)}journalctl -u ${m[2]} -n 50 --no-pager`] },
  { re: /^(sudo )?systemctl status (\S+)$/, next: (m) => [`${S(m)}journalctl -u ${m[2]} -n 50 --no-pager`, `${S(m)}systemctl restart ${m[2]}`] },
  { re: /^(sudo )?systemctl (?:enable|disable) (\S+)$/, next: (m) => [`${S(m)}systemctl status ${m[2]}`] },
  { re: /^(sudo )?apt(?:-get)? update$/, next: (m) => ["apt list --upgradable", `${S(m)}apt upgrade`] },
  { re: /^(sudo )?apt(?:-get)? (?:upgrade|full-upgrade|dist-upgrade)(?: -y)?$/, next: (m) => [`${S(m)}apt autoremove`, "cat /var/run/reboot-required"] },
  { re: /^(sudo )?apt(?:-get)? install (?:-y )?(\S+)$/, next: (m) => [`systemctl status ${m[2]}`] },
  { re: /^(sudo )?dnf (?:upgrade|update)(?: -y)?$/, next: () => ["needs-restarting -r"] },
  { re: /^(sudo )?ufw (?:allow|deny|limit|delete|reject) .+$/, next: (m) => [`${S(m)}ufw status numbered`] },
  { re: /^(sudo )?ufw (?:enable|reload)$/, next: (m) => [`${S(m)}ufw status verbose`] },
  { re: /^(sudo )?certbot (?:renew|certonly|--nginx|--apache).*$/, next: (m) => [`${S(m)}certbot certificates`] },
  { re: /^(sudo )?docker compose pull$/, next: (m) => [`${S(m)}docker compose up -d`] },
  { re: /^(sudo )?docker compose (?:up|restart)\b.*$/, next: (m) => [`${S(m)}docker compose ps`, `${S(m)}docker compose logs -f --tail=100`] },
  { re: /^(sudo )?docker compose down$/, next: (m) => [`${S(m)}docker compose up -d`] },
  { re: /^(sudo )?docker (?:restart|start|run -d) .*?(\S+)$/, next: (m) => [`${S(m)}docker ps`, `${S(m)}docker logs -f --tail=100 ${m[2]}`] },
  { re: /^git pull\b.*$/, next: () => ["git log --oneline -5", "git status"] },
  { re: /^git clone (?:\S+ )*?(\S+?)(?:\.git)?\/?$/, next: (m) => [`cd ${m[1].split(/[/:]/).pop()}`] },
  { re: /^git (?:add|commit)\b.*$/, next: () => ["git status", "git push"] },
  { re: /^git status$/, next: () => ["git diff", "git pull"] },
  { re: /^(?:npm|pnpm|yarn) (?:ci|install|i)$/, next: (m) => [`${m[0].split(" ")[0]} run build`] },
  { re: /^mkdir (?:-p )?(\S+)$/, next: (m) => [`cd ${m[1]}`] },
  { re: /^cd (?!-$)(\S+)$/, next: () => ["ls -la"] },
  { re: /^tar (?:-?[xz]\S*) .*$/, next: () => ["ls -la"] },
  { re: /^df -h$/, next: () => ["du -xh --max-depth=1 / 2>/dev/null | sort -h | tail -15"] },
  { re: /^(sudo )?crontab -e$/, next: (m) => [`${S(m)}crontab -l`] },
  { re: /^ssh-keygen\b.*$/, next: () => ["cat ~/.ssh/id_ed25519.pub"] },
];

/** Common next steps after `last` (exact matches of the rules above). */
export function nextSteps(last: string | undefined): string[] {
  const cmd = last?.trim();
  if (!cmd) return [];
  for (const r of RULES) {
    const m = cmd.match(r.re);
    if (m) return r.next(m);
  }
  return [];
}

export interface SuggestContext {
  /** Commands typed in this terminal, oldest first. */
  session: string[];
  /** The server's shell history, newest first. */
  history: string[];
}

export const MAX_SUGGESTIONS = 4;

/**
 * What to suggest for the line typed so far. An empty line gets the next
 * steps after the last command; a started line gets commands that begin with
 * it: next steps first, then this session's commands, then the history.
 * A line starting with a space gets nothing (shells keep those out of history).
 */
export function suggest(typed: string, ctx: SuggestContext): string[] {
  const last = ctx.session[ctx.session.length - 1];
  if (typed.startsWith(" ")) return [];
  const out: string[] = [];
  const add = (c: string) => {
    if (out.length < MAX_SUGGESTIONS && c !== typed && c.startsWith(typed) && !out.includes(c)) out.push(c);
  };
  nextSteps(last).forEach(add);
  if (!typed) return out;
  [...ctx.session].reverse().forEach(add);
  ctx.history.forEach(add);
  return out;
}
