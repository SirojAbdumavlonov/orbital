// "What's new": the release notes Orbital shows on its main page and, once,
// after an update. They come from CHANGELOG.md, bundled into the app at build
// time, so they work offline and always match the version that's running.
// Everything here is pure and tested (whatsNew.test.ts); the text is shown as
// plain text, never as HTML.

export type ChangeKind = "added" | "changed" | "fixed" | "other";

export interface ChangeItem {
  kind: ChangeKind;
  /** The bold lead of the bullet ("Test a website"), or its first words when it has none. */
  title: string;
  /** One or two sentences for lists: the first sentence after the title. */
  summary: string;
  /** The whole entry as plain paragraphs and sub-points. */
  details: string[];
}

export interface Release {
  version: string;
  date?: string;
  items: ChangeItem[];
  /** Where the full notes are on GitHub. */
  url: string;
}

import { REPO_URL } from "./links";

const KINDS: Record<string, ChangeKind> = { added: "added", changed: "changed", fixed: "fixed", security: "fixed", improved: "changed" };
export const KIND_LABEL: Record<ChangeKind, string> = { added: "New", changed: "Improved", fixed: "Fixed", other: "Other" };

/** Markdown emphasis and code marks removed; links become their text. */
export const plain = (s: string) =>
  s
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/\*\*(.+?)\*\*/g, "$1")
    .replace(/`([^`]+)`/g, "$1")
    .replace(/\s+/g, " ")
    .trim();

/** GitHub's anchor for a heading such as "1.4.0 (2026-11-22)" → "140-2026-11-22". */
export const anchor = (heading: string) =>
  heading.trim().toLowerCase().replace(/[^\p{L}\p{N}\s_-]/gu, "").replace(/\s/g, "-");

const MAX_SUMMARY = 150;
/** Text ending in one of these hasn't ended its sentence. */
const ABBREV = /\b(?:e\.g|i\.e|etc|vs|incl)\.$/i;

/** The first sentence of `text`, cut at a word near MAX_SUMMARY when it is long. */
export function firstSentence(text: string): string {
  const t = text.replace(/^[\s:.,;—–-]+/, "").trim();
  // A sentence ends at ". " (or "! ?") followed by a capital or a quote, outside parentheses.
  let depth = 0;
  let end = t.length;
  for (let i = 0; i < t.length; i++) {
    const c = t[i];
    if (c === "(") depth++;
    else if (c === ")") depth = Math.max(0, depth - 1);
    else if (depth === 0 && /[.!?]/.test(c) && /\s/.test(t[i + 1] ?? "") && /["“A-Z0-9]/.test(t[i + 2] ?? "") && !ABBREV.test(t.slice(0, i + 1))) {
      end = i + 1;
      break;
    }
  }
  const s = t.slice(0, end).trim();
  if (s.length <= MAX_SUMMARY) return s;
  const cut = s.slice(0, MAX_SUMMARY);
  return cut.slice(0, Math.max(cut.lastIndexOf(" "), 60)).replace(/[,;:(—–-]+$/, "").trimEnd() + "…";
}

function item(kind: ChangeKind, lines: string[]): ChangeItem {
  // lines[0] is the bullet's text; the rest are its continuation lines and sub-points.
  const head = lines[0];
  const bold = /^\*\*(.+?)\*\*\s*(.*)$/.exec(head);
  let title: string;
  let rest: string;
  if (bold) {
    title = plain(bold[1]).replace(/[.:]$/, "");
    rest = bold[2];
  } else {
    const p = plain(head);
    const words = p.split(" ");
    title = words.length > 8 ? words.slice(0, 8).join(" ") + "…" : p.replace(/[.:]$/, "");
    rest = words.length > 8 ? p : "";
  }
  const details: string[] = [];
  let para = rest;
  const flush = () => {
    if (plain(para)) details.push(plain(para));
    para = "";
  };
  for (const l of lines.slice(1)) {
    const sub = /^\s*[-*]\s+(.*)/.exec(l);
    if (sub) {
      flush();
      para = "• " + sub[1];
    } else if (!l.trim()) flush();
    else para += " " + l.trim();
  }
  flush();
  // The summary skips a leading "(where to find it)" and an intro to a list
  // ("…:"), preferring a paragraph over a sub-point.
  const usable = (t: string) => firstSentence(t.replace(/^\([^)]*\)\s*/, "").replace(/^•\s*/, ""));
  const candidates = [plain(rest), ...details.filter((d) => !d.startsWith("•")), ...details.filter((d) => d.startsWith("•"))];
  const summary = candidates.map(usable).find((c) => c && !c.endsWith(":")) ?? "";
  return { kind, title, summary, details };
}

/** CHANGELOG.md → releases, newest first. Empty sections (such as an empty "Unreleased") are left out. */
export function parseChangelog(md: string): Release[] {
  const out: Release[] = [];
  let rel: Release | null = null;
  let kind: ChangeKind = "other";
  let cur: string[] | null = null;
  const close = () => {
    if (rel && cur) rel.items.push(item(kind, cur));
    cur = null;
  };
  for (const line of md.split("\n")) {
    const h2 = /^##\s+(.+?)\s*$/.exec(line);
    if (h2) {
      close();
      const m = /^\[?v?(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)\]?(?:\s*\(([^)]+)\))?/.exec(h2[1]);
      rel = m ? { version: m[1], date: m[2], items: [], url: `${REPO_URL}/blob/main/CHANGELOG.md#${anchor(h2[1])}` } : null;
      if (rel) out.push(rel);
      kind = "other";
      continue;
    }
    if (!rel) continue;
    const h3 = /^###\s+(.+?)\s*$/.exec(line);
    if (h3) {
      close();
      kind = KINDS[h3[1].toLowerCase()] ?? "other";
      continue;
    }
    const bullet = /^[-*]\s+(.*)/.exec(line);
    if (bullet) {
      close();
      cur = [bullet[1]];
    } else if (cur) cur.push(line);
  }
  close();
  return out.filter((r) => r.items.length);
}

/** Compares "1.4.0", "1.4.0-beta.2": numbers by value, a pre-release before its release. */
export function compareVersions(a: string, b: string): number {
  const [ma, pa = ""] = a.replace(/^v/, "").split("-", 2);
  const [mb, pb = ""] = b.replace(/^v/, "").split("-", 2);
  const na = ma.split(".").map(Number);
  const nb = mb.split(".").map(Number);
  for (let i = 0; i < 3; i++) {
    const d = (na[i] || 0) - (nb[i] || 0);
    if (d) return Math.sign(d);
  }
  if (pa === pb) return 0;
  if (!pa) return 1;
  if (!pb) return -1;
  return pa.localeCompare(pb, undefined, { numeric: true });
}

/** Releases newer than `seen`, up to and including `current`, newest first. */
export function releasesSince(all: Release[], seen: string, current: string): Release[] {
  return all.filter((r) => compareVersions(r.version, seen) > 0 && compareVersions(r.version, current) <= 0);
}

/** The notes for the running version, or the newest older one when it has none of its own. */
export function currentRelease(all: Release[], current: string): Release | undefined {
  return all.find((r) => compareVersions(r.version, current) <= 0);
}

/**
 * Whether to open the "What's new" dialog: only after an update. A first
 * launch (nothing seen yet) and a downgrade show nothing.
 */
export function shouldAnnounce(seen: string | null, current: string, all: Release[]): boolean {
  return !!seen && compareVersions(current, seen) > 0 && releasesSince(all, seen, current).length > 0;
}

/**
 * What to announce at launch: the releases since the last version that ran,
 * newest first (at most five), or null for nothing. With no version recorded,
 * `usedBefore` (Orbital's other local data exists) means an update from a
 * version that didn't record it, which gets the current version's notes; a
 * fresh install gets nothing.
 */
export function announcement(seen: string | null, current: string, all: Release[], usedBefore: boolean): Release[] | null {
  if (seen) return shouldAnnounce(seen, current, all) ? releasesSince(all, seen, current).slice(0, 5) : null;
  const rel = currentRelease(all, current);
  return usedBefore && rel?.version === current ? [rel] : null;
}
