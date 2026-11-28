import { Check, KeyRound, Plus, RefreshCw } from "lucide-react";
import { useEffect, useState } from "react";
import { api, asApiError, LocalKey } from "../lib/api";
import { NewKeyDialog } from "../views/SshKeysView";

// Under a "public keys" box (New user): the public keys in ~/.ssh on this
// computer, added with one click, and a new key pair made right here, so
// nobody has to open a terminal or another app to find or make one.

/** Adds `line` to the newline-separated `text` unless that key is there already (same type and data). Pure; tested. */
export function withKeyLine(text: string, line: string): string {
  const key = (l: string) => l.trim().split(/\s+/).slice(0, 2).join(" ");
  const lines = text.split("\n").map((l) => l.trim()).filter(Boolean);
  if (lines.some((l) => key(l) === key(line))) return lines.join("\n");
  return [...lines, line.trim()].join("\n");
}

/** Drops every line holding this key (whatever its comment). Pure; tested. */
export function withoutKeyLine(text: string, line: string): string {
  return text.split("\n").filter((l) => l.trim() && !hasKeyLine(l, line)).join("\n");
}

/** Is this key (type and data) in the text? */
export const hasKeyLine = (text: string, line: string) => withKeyLine(text, line) === text.split("\n").map((l) => l.trim()).filter(Boolean).join("\n");

export function LocalKeyPicker({ value, onChange, forUser, server }: { value: string; onChange: (v: string) => void; forUser: string; server: string }) {
  const [keys, setKeys] = useState<LocalKey[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [making, setMaking] = useState(false);
  const load = () => api.localSshKeys().then((k) => { setKeys(k); setError(null); }, (e) => setError(asApiError(e).message));
  useEffect(() => { void load(); }, []);
  const user = forUser.trim() || "user";
  return (
    <div className="key-picker">
      <div className="key-picker-head">
        <span className="faint">On this computer (~/.ssh):</span>
        <span className="spacer" />
        <button className="btn ghost" onClick={() => setMaking(true)}><Plus size={12} /> New key{forUser.trim() ? ` for ${user}` : ""}</button>
        <button className="icon-btn" aria-label="Look in ~/.ssh again" title="Look in ~/.ssh again" onClick={() => void load()}><RefreshCw size={13} /></button>
      </div>
      {error && <small className="bad-text">{error}</small>}
      {keys && keys.length === 0 && <small className="faint">No keys yet. New key makes one here (the private half stays on this computer).</small>}
      {keys && keys.length > 0 && (
        <div className="key-chips">
          {keys.map((k) => {
            const on = hasKeyLine(value, k.publicLine);
            return (
              <button key={k.path} className={`chip ${on ? "on" : ""}`} title={`${k.algorithm} · ${k.fingerprint}${k.comment ? ` · ${k.comment}` : ""}`}
                onClick={() => onChange(on ? withoutKeyLine(value, k.publicLine) : withKeyLine(value, k.publicLine))}>
                {on ? <Check size={11} /> : <KeyRound size={11} />} {k.name}{k.comment ? <span className="faint"> {k.comment}</span> : null}
              </button>
            );
          })}
        </div>
      )}
      {making && (
        <NewKeyDialog
          existing={(keys ?? []).map((k) => k.name)}
          base={`id_ed25519_${user.replace(/[^A-Za-z0-9_-]/g, "") || "user"}`}
          defaultComment={`${user}@${server}`}
          subtitle={`An Ed25519 key pair for ${user}, made on this computer in ~/.ssh. Its public half is added to this user; give the private half to the person who signs in, or keep it to sign in as ${user} from Orbital.`}
          onClose={() => setMaking(false)}
          onMade={(k) => { setMaking(false); onChange(withKeyLine(value, k.publicLine)); void load(); }}
        />
      )}
    </div>
  );
}
