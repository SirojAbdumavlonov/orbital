import { useEffect, useState } from "react";
import { KEYS, mod } from "../lib/platform";
import { Modal } from "./ui";

/** Every keyboard shortcut, as this platform writes them (Help page and the ⌘/ sheet). */
export function ShortcutTable() {
  return (
    <table className="help-keys">
      <tbody>
        <tr><td><kbd>{KEYS.commandMenu}</kbd></td><td>Command menu: jump to any server, tab or page</td></tr>
        <tr><td><kbd>{KEYS.newServer}</kbd></td><td>Add a server</td></tr>
        <tr><td><kbd>{KEYS.back}</kbd> <kbd>{KEYS.forward}</kbd></td><td>Back and forward between places</td></tr>
        <tr><td><kbd>{KEYS.sidebar}</kbd></td><td>Show or hide the sidebar</td></tr>
        <tr><td><kbd>{KEYS.pane}</kbd></td><td>Hide or show the right-hand pane (the editor in Files, the details panel in Services, Docker…) to give the list the whole width</td></tr>
        <tr><td><kbd>{KEYS.copy}</kbd> <kbd>{KEYS.paste}</kbd></td><td>In Files: copy the selected file or folder, and paste here files copied in Orbital (same or another server) or on this computer, or a copied screenshot</td></tr>
        <tr><td><kbd>{KEYS.zoomIn}</kbd> <kbd>{KEYS.zoomOut}</kbd> <kbd>{KEYS.zoomReset}</kbd></td><td>Bigger, smaller, back to normal: the terminal's text in a terminal, the editor's in the editor, the whole interface anywhere else</td></tr>
        <tr><td><kbd>{KEYS.gotoFolder}</kbd></td><td>In Files: go to a folder by typing its path</td></tr>
        <tr><td><kbd>→</kbd></td><td>In the terminal, at the end of a line: accept the suggestion</td></tr>
        <tr><td><kbd>{KEYS.shortcuts}</kbd></td><td>This list</td></tr>
      </tbody>
    </table>
  );
}

/** Mounted once in App: ⌘/ (Ctrl+/) shows the shortcuts anywhere, except while typing in the terminal or editor. */
export function ShortcutSheet() {
  const [open, setOpen] = useState(false);
  useEffect(() => {
    const on = (e: KeyboardEvent) => {
      if (e.key !== "/" || !mod(e) || e.altKey || e.shiftKey) return;
      const t = e.target as HTMLElement | null;
      if (typeof t?.closest === "function" && t.closest(".xterm, .monaco-editor")) return;
      e.preventDefault();
      setOpen((o) => !o);
    };
    window.addEventListener("keydown", on);
    return () => window.removeEventListener("keydown", on);
  }, []);
  if (!open) return null;
  return (
    <Modal title="Keyboard shortcuts" onClose={() => setOpen(false)}>
      <ShortcutTable />
    </Modal>
  );
}
