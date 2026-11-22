import { ChevronDown, ChevronRight, ExternalLink, MessageSquare, Sparkles, X } from "lucide-react";
import { useEffect, useState } from "react";
import { create } from "zustand";
import changelog from "../../CHANGELOG.md?raw";
import pkg from "../../package.json";
import { announcement, ChangeItem, currentRelease, KIND_LABEL, parseChangelog, Release } from "../lib/whatsNew";
import { useStatsAsking } from "./StatsPrompt";
import { Modal } from "./ui";
import { openRepoUrl, REPO_URL } from "../lib/links";
import { useStore } from "../store";
import { feedbackAbout } from "../views/FeedbackView";

/** Every release in CHANGELOG.md, newest first, bundled at build time. */
export const RELEASES = parseChangelog(changelog);

/** The last version whose notes were shown (or that ran), so an update can be announced once. */
const SEEN_KEY = "orbital.seenVersion";
const seen = () => {
  try { return localStorage.getItem(SEEN_KEY); } catch { return null; }
};
const markSeen = (v: string) => {
  try { localStorage.setItem(SEEN_KEY, v); } catch { /* private mode: announced again next time */ }
};

/**
 * The version whose changes the main page shows: set by an update, cleared by
 * the card's ×. A fresh install shows no card; the Changes page has them all.
 */
const CARD_KEY = "orbital.whatsNewCard";
const useCard = create<{ version: string | null; showFor(v: string): void; close(): void }>((set) => ({
  version: (() => { try { return localStorage.getItem(CARD_KEY); } catch { return null; } })(),
  showFor(v) {
    try { localStorage.setItem(CARD_KEY, v); } catch { /* shown for this run only */ }
    set({ version: v });
  },
  close() {
    try { localStorage.removeItem(CARD_KEY); } catch { /* hidden for this run only */ }
    set({ version: null });
  },
}));

/**
 * Versions before 1.4.1 didn't record which version ran, so an update from
 * one looks like a first launch. Orbital's other local storage (settings,
 * tabs, bookmarks, panel sizes) tells them apart: a fresh install has none.
 */
const usedBefore = (() => {
  // Taken when the app starts, before anything this run writes (the stats question's flag, say).
  try {
    return Object.keys(localStorage).some((k) => k.startsWith("orbital.") && ![SEEN_KEY, CARD_KEY, "orbital.statsAsked"].includes(k));
  } catch {
    return false;
  }
})();

/** The running app's version (package.json's outside the app, which the release script keeps equal). */
export const useAppVersion = create<{ version: string; load(): Promise<void> }>((set) => ({
  version: pkg.version,
  async load() {
    try {
      const { getVersion } = await import("@tauri-apps/api/app");
      set({ version: await getVersion() });
    } catch { /* plain browser: keep package.json's */ }
  },
}));

interface DialogState {
  /** The releases on show, newest first; null when closed. */
  releases: Release[] | null;
  /** "update": right after an update; "browse": opened by hand. */
  mode: "update" | "browse";
  show(releases: Release[], mode?: "update" | "browse"): void;
  close(): void;
}

export const useWhatsNew = create<DialogState>((set, get) => ({
  releases: null,
  mode: "browse",
  show: (releases, mode = "browse") => set({ releases, mode }),
  close: () => {
    if (get().mode === "update") markSeen(useAppVersion.getState().version);
    set({ releases: null });
  },
}));

/** Opens the notes: one release, or all of them. */
export const showWhatsNew = (release?: Release) => useWhatsNew.getState().show(release ? [release] : RELEASES);

const kindBadge: Record<ChangeItem["kind"], string> = { added: "amber", changed: "", fixed: "ok", other: "" };

function Item({ item, version, open: startOpen }: { item: ChangeItem; version: string; open?: boolean }) {
  const [open, setOpen] = useState(!!startOpen);
  const more = item.details.length > 0 && item.details.join(" ") !== item.summary;
  return (
    <li className="wn-item">
      <span className={`badge ${kindBadge[item.kind]}`}>{KIND_LABEL[item.kind]}</span>
      <div className="wn-text">
        <b>{item.title}</b>
        {!open && item.summary && <span>{item.summary}</span>}
        {open && item.details.map((d, i) => <p key={i} className={d.startsWith("•") ? "wn-sub" : ""}>{d.replace(/^•\s*/, "")}</p>)}
        <div className="wn-actions">
          {more && (
            <button className="wn-link" onClick={() => setOpen(!open)}>
              {open ? <ChevronDown size={12} /> : <ChevronRight size={12} />} {open ? "Less" : "More"}
            </button>
          )}
          <button className="wn-link" title="Tell us what you think of this change, or report a problem with it" onClick={() => { useWhatsNew.getState().close(); feedbackAbout(`${version}: ${item.title}`, item.kind === "fixed" ? "problem" : "idea"); }}>
            <MessageSquare size={11} /> Feedback
          </button>
        </div>
      </div>
    </li>
  );
}

function ReleaseNotes({ release }: { release: Release }) {
  return (
    <section className="wn-release">
      <header>
        <h3>Orbital {release.version}</h3>
        {release.date && <span className="faint">{release.date}</span>}
        <button className="wn-link" onClick={() => openRepoUrl(release.url)}>Full notes <ExternalLink size={11} /></button>
      </header>
      <ul>{release.items.map((it, i) => <Item key={i} item={it} version={release.version} />)}</ul>
    </section>
  );
}

/** The dialog in the middle of the window: after an update, or from the main page, ⌘K and Settings. */
export function WhatsNewDialog() {
  const { releases, mode, close } = useWhatsNew();
  const statsAsking = useStatsAsking();
  if (!releases || statsAsking) return null;
  const one = releases.length === 1;
  const title = mode === "update"
    ? `Orbital is updated to ${releases[0].version}`
    : one ? `What's new in Orbital ${releases[0].version}` : "What's new in Orbital";
  const subtitle = mode === "update"
    ? releases.length > 1 ? `Here's what changed since the version you had before (${releases.length} releases).` : "Here's what changed."
    : undefined;
  return (
    <Modal
      title={title}
      subtitle={subtitle}
      wide
      onClose={close}
      footer={
        <>
          <button className="btn" onClick={() => openRepoUrl(`${REPO_URL}/blob/main/CHANGELOG.md`)}><ExternalLink size={13} /> All release notes on GitHub</button>
          <button className="btn primary" onClick={close}>{mode === "update" ? "Got it" : "Close"}</button>
        </>
      }
    >
      <div className="wn-list">{releases.map((r) => <ReleaseNotes key={r.version} release={r} />)}</div>
    </Modal>
  );
}

/**
 * Mounted once in App: after an update, opens the dialog with the notes of
 * every version since the last one that ran (at most five). A first launch
 * only remembers the version.
 */
export function useWhatsNewAnnounce() {
  useEffect(() => {
    void (async () => {
      await useAppVersion.getState().load();
      const v = useAppVersion.getState().version;
      const list = announcement(seen(), v, RELEASES, usedBefore);
      if (list) {
        useWhatsNew.getState().show(list, "update");
        useCard.getState().showFor(v);
      } else markSeen(v);
    })();
  }, []);
}

/** The main page's card after an update: the new version's changes, with × to remove it. */
export function WhatsNewCard() {
  const version = useAppVersion((s) => s.version);
  const rel = currentRelease(RELEASES, version);
  const card = useCard();
  // Only after an update to this version, until the × closes it.
  if (!rel || card.version !== rel.version) return null;
  const shown = rel.items.slice(0, 4);
  return (
    <div className="wn-card">
      <div className="wn-card-head">
        <Sparkles size={14} />
        <b>What's new in {rel.version}</b>
        {rel.date && <span className="faint">{rel.date}</span>}
        <button className="wn-close" title="Remove from the main page (every version's changes stay in Changes)" aria-label="Remove what's new from the main page" onClick={card.close}><X size={13} /></button>
      </div>
      <ul>
        {shown.map((it, i) => (
          <li key={i}>
            <button onClick={() => showWhatsNew(rel)}>
              <span className={`badge ${kindBadge[it.kind]}`}>{KIND_LABEL[it.kind]}</span>
              <span className="wn-card-text"><b>{it.title}</b>{it.summary && <span>{it.summary}</span>}</span>
            </button>
          </li>
        ))}
      </ul>
      <div className="wn-card-foot">
        <button className="wn-link" onClick={() => showWhatsNew(rel)}>
          {rel.items.length > shown.length ? `See all ${rel.items.length} changes` : "Details"}
        </button>
        <button className="wn-link" onClick={() => useStore.getState().setGlobalView("changes")}>Earlier versions</button>
      </div>
    </div>
  );
}

/** The Changes page (sidebar): every version's notes, newest first. */
export function ChangesView() {
  return (
    <div className="view">
      <div className="scroll">
        <div className="settings wn-page">
          {RELEASES.map((r) => <ReleaseNotes key={r.version} release={r} />)}
        </div>
      </div>
    </div>
  );
}
