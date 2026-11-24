import { NotebookPen } from "lucide-react";
import { useEffect, useState } from "react";
import { relativeTime } from "../lib/format";
import { useNotes } from "../lib/notes";
import { SessionState } from "../store";

/** Notes about this server: what runs where, how to deploy, who to call. Saved on this computer as you type. */
export function NotesView({ s }: { s: SessionState }) {
  const pid = s.profile.id;
  const note = useNotes((x) => x.byProfile[pid]);
  useEffect(() => { useNotes.getState().get(pid); }, [pid]);
  const [text, setText] = useState<string | null>(null);
  useEffect(() => { if (note && text === null) setText(note.text); }, [note]);
  // Saved half a second after you stop typing.
  useEffect(() => {
    if (text === null || text === note?.text) return;
    const t = setTimeout(() => useNotes.getState().set(pid, text), 500);
    return () => clearTimeout(t);
  }, [text]);
  return (
    <div className="view notes-view">
      <div className="notes-head">
        <NotebookPen size={15} className="faint" />
        <b>Notes about {s.profile.name}</b>
        <span className="spacer" />
        <span className="faint">{note?.updated ? `Saved ${relativeTime(Math.floor(note.updated / 1000))}` : "Saved on this computer as you type"}</span>
      </div>
      <textarea
        className="notes-text"
        value={text ?? ""}
        onChange={(e) => setText(e.target.value)}
        placeholder={"What runs on this server, how to deploy it, where the logs are, who to call…\n\nFor example:\n# Shop API\n- deploy: Deploy tab → shop-api\n- logs: /var/log/shop/api.log\n- on call: ops team"}
        spellCheck
      />
      <p className="faint notes-hint">Kept only on this computer, not encrypted: don't keep passwords or keys here (Orbital keeps those in its encrypted vault).</p>
    </div>
  );
}
