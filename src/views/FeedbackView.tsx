import { Bug, CheckCircle2, ExternalLink, Heart, Lightbulb, MessageSquare, Send } from "lucide-react";
import { useEffect, useState } from "react";
import { create } from "zustand";
import { api, asApiError, FeedbackKind } from "../lib/api";
import { issueUrl, openRepoUrl } from "../lib/links";
import { useAppVersion } from "../components/WhatsNew";
import { useStore } from "../store";

// The Feedback page: a problem, an idea or a word of thanks, sent to the
// project's own server (stats-server/, the same place as the usage numbers)
// when the user clicks Send. Only what they type here plus the Orbital
// version and OS leave this computer. Builds without that server (development)
// offer a pre-filled GitHub issue instead.

const MAX_MESSAGE = 4000;
const MAX_CONTACT = 200;

/** The parts of Orbital people usually write about; a change from the Changes page adds its own. */
export const FEEDBACK_AREAS = [
  "Orbital in general", "Connecting and servers", "Files", "Editor and safe save", "Terminal", "Monitor", "Services",
  "Docker", "Moving containers", "Nginx and websites", "HTTPS certificates", "Domains (DNS)", "Databases",
  "Firewall and security", "Users and SSH keys", "Cron", "Logs", "Port forwarding", "Deploy", "Uptime",
  "AI agent (Telegram)", "Snippets", "Updates and installing", "Something else",
];

const KINDS: { id: FeedbackKind; label: string; icon: typeof Bug; placeholder: string }[] = [
  { id: "problem", label: "Something's wrong", icon: Bug, placeholder: "What did you do, what happened, and what did you expect? An error message helps a lot." },
  { id: "idea", label: "An idea", icon: Lightbulb, placeholder: "What would you like Orbital to do, or do differently? What would it help you with?" },
  { id: "praise", label: "Something I like", icon: Heart, placeholder: "What works well for you? It helps to know what to keep." },
  { id: "other", label: "Something else", icon: MessageSquare, placeholder: "Anything you'd like to tell us." },
];

interface Draft {
  kind: FeedbackKind;
  about: string;
  message: string;
  contact: string;
  set(p: Partial<Omit<Draft, "set">>): void;
}

/** The draft survives switching pages while Orbital is open. */
export const useFeedbackDraft = create<Draft>((set) => ({
  kind: "idea",
  about: FEEDBACK_AREAS[0],
  message: "",
  contact: "",
  set: (p) => set(p),
}));

/** Opens the Feedback page about one thing (a change from the Changes page, say). */
export function feedbackAbout(about: string, kind: FeedbackKind = "idea") {
  useFeedbackDraft.getState().set({ about: about.slice(0, 80), kind });
  useStore.getState().setGlobalView("feedback");
}

export function FeedbackView() {
  const d = useFeedbackDraft();
  const version = useAppVersion((s) => s.version);
  const [available, setAvailable] = useState<boolean | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [sent, setSent] = useState(false);
  useEffect(() => { api.feedbackAvailable().then(setAvailable).catch(() => setAvailable(false)); }, []);

  const kind = KINDS.find((k) => k.id === d.kind) ?? KINDS[1];
  const areas = FEEDBACK_AREAS.includes(d.about) ? FEEDBACK_AREAS : [d.about, ...FEEDBACK_AREAS];
  const message = d.message.trim();
  const os = navigator.userAgent.includes("Mac") ? "macOS" : navigator.userAgent.includes("Windows") ? "Windows" : "Linux";

  const send = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.sendFeedback({ kind: d.kind, about: d.about, message, contact: d.contact.trim() });
      setSent(true);
      d.set({ message: "", about: FEEDBACK_AREAS[0] });
    } catch (e) {
      setError(asApiError(e).message);
    } finally {
      setBusy(false);
    }
  };

  if (sent) {
    return (
      <div className="view"><div className="scroll"><div className="settings feedback">
        <div className="feedback-sent">
          <CheckCircle2 size={28} />
          <h3>Thank you, it's sent</h3>
          <p className="faint">{d.contact.trim() ? `If there's a question or an answer, it goes to ${d.contact.trim()}.` : "Every message is read."}</p>
          <button className="btn" onClick={() => setSent(false)}>Write another</button>
        </div>
      </div></div></div>
    );
  }

  return (
    <div className="view">
      <div className="scroll">
        <div className="settings feedback">
          <p className="feedback-lead">
            Tell us what doesn't work, what's missing or what you'd change. It goes straight to the people who make Orbital.
          </p>

          <div className="feedback-kinds" role="radiogroup" aria-label="What is it?">
            {KINDS.map((k) => (
              <button key={k.id} role="radio" aria-checked={d.kind === k.id} className={`feedback-kind ${d.kind === k.id ? "on" : ""}`} onClick={() => d.set({ kind: k.id })}>
                <k.icon size={16} /> {k.label}
              </button>
            ))}
          </div>

          <label className="field">
            <span>About</span>
            <select className="input" value={d.about} onChange={(e) => d.set({ about: e.target.value })}>
              {areas.map((a) => <option key={a} value={a}>{a}</option>)}
            </select>
          </label>

          <label className="field">
            <span>Your message</span>
            <textarea
              className="input"
              rows={8}
              maxLength={MAX_MESSAGE}
              value={d.message}
              placeholder={kind.placeholder}
              onChange={(e) => d.set({ message: e.target.value })}
            />
            <span className="hint">
              {d.message.length > MAX_MESSAGE - 400 ? `${MAX_MESSAGE - d.message.length} characters left. ` : ""}
              Please leave out passwords, keys, server addresses and other private details.
            </span>
          </label>

          <label className="field">
            <span>How to reach you (optional)</span>
            <input className="input" maxLength={MAX_CONTACT} value={d.contact} placeholder="Email or Telegram, if you'd like an answer" onChange={(e) => d.set({ contact: e.target.value })} />
          </label>

          {error && <div className="feedback-error">{error}</div>}

          <div className="feedback-foot">
            <small className="faint">
              {available === false
                ? "This copy of Orbital can't send feedback itself. Open it as a GitHub issue instead (public; your contact is left out)."
                : `Sent: what you typed here, Orbital ${version} and your system (${os}). Nothing about your servers, and it doesn't depend on the usage statistics setting.`}
            </small>
            {available === false ? (
              <button className="btn primary" disabled={!message} onClick={() => openRepoUrl(issueUrl(d.kind, d.about, message, version, os))}>
                <ExternalLink size={13} /> Open a GitHub issue
              </button>
            ) : (
              <button className="btn primary" disabled={!message || busy || available === null} onClick={send}>
                <Send size={13} /> {busy ? "Sending…" : "Send"}
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
