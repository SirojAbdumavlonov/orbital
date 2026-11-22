// Copy and paste in Files (⌘C / ⌘V, Ctrl+C / Ctrl+V): pure decisions, tested
// in paste.test.ts. FilesView does the work.
//
// ⌘C on a file in Orbital remembers it here and also puts its path on the
// system clipboard as text. ⌘V then decides from the system clipboard, so the
// most recent copy wins: files copied in Finder / Explorer / a file manager
// are uploaded; Orbital's own copy is pasted when the clipboard still holds its
// path; a copied image (a screenshot) is uploaded as a PNG.

import { create } from "zustand";

/** A file or folder copied in Orbital's Files. */
export interface CopiedEntry {
  sid: string;
  path: string;
  name: string;
  dir: boolean;
  /** The server's name, for messages. */
  server: string;
}

export const useCopied = create<{ entry: CopiedEntry | null; set(e: CopiedEntry | null): void }>((set) => ({
  entry: null,
  set: (entry) => set({ entry }),
}));

/** What the system clipboard holds (the `clipboard_read` command). */
export interface SystemClipboard {
  files: string[];
  text: string | null;
  image: string | null;
}

export type PasteAction =
  | { kind: "upload"; paths: string[] }
  | { kind: "image"; path: string }
  | { kind: "copySame"; from: CopiedEntry }
  | { kind: "copyOther"; from: CopiedEntry }
  | { kind: "nothing"; why: string };

export function pasteAction(sys: SystemClipboard, copied: CopiedEntry | null, here: { sid: string }): PasteAction {
  if (sys.files.length) return { kind: "upload", paths: sys.files };
  // Orbital's copy counts while the clipboard still holds its path (or nothing we can read).
  if (copied && (sys.text === null || sys.text.trim() === copied.path)) {
    return copied.sid === here.sid ? { kind: "copySame", from: copied } : { kind: "copyOther", from: copied };
  }
  if (sys.image) return { kind: "image", path: sys.image };
  return { kind: "nothing", why: "Copy files on this computer, or a file in Orbital, then paste them here." };
}

/** A name not taken in the folder: "a.txt" → "a copy.txt" → "a copy 2.txt", like Finder. Folders and dotfiles keep their whole name. */
export function freeName(name: string, taken: Iterable<string>, dir = false): string {
  const used = new Set(taken);
  if (!used.has(name)) return name;
  const dot = dir ? -1 : name.lastIndexOf(".");
  const [base, ext] = dot > 0 ? [name.slice(0, dot), name.slice(dot)] : [name, ""];
  for (let i = 1; ; i++) {
    const n = `${base} copy${i === 1 ? "" : ` ${i}`}${ext}`;
    if (!used.has(n)) return n;
  }
}

/** The last part of a local path, on any system. */
export const baseName = (p: string) => p.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || p;

/** Keys typed here are someone else's: text fields, the editor, the terminal. */
export function typingIn(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  if (!el || typeof el.closest !== "function") return false;
  return !!el.closest("input, textarea, select, [contenteditable=''], [contenteditable='true'], .monaco-editor, .xterm");
}
