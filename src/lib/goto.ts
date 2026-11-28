// Go to Folder (Files, ⌘⇧G): turning what the user typed into a path on the
// server, and splitting it for suggestions. Pure; tested.

/** An absolute, tidy path from what was typed: `~` is the home folder, a relative path starts at `cwd`, `.` and `..` are followed. */
export function resolvePath(input: string, cwd: string, home: string): string {
  let p = input.trim();
  if (!p) return cwd;
  if (p === "~" || p.startsWith("~/")) p = home.replace(/\/+$/, "") + p.slice(1);
  else if (!p.startsWith("/")) p = `${cwd.replace(/\/+$/, "")}/${p}`;
  const parts: string[] = [];
  for (const part of p.split("/")) {
    if (!part || part === ".") continue;
    if (part === "..") parts.pop();
    else parts.push(part);
  }
  return "/" + parts.join("/");
}

/** The folder to list and the start of the name being typed: "/etc/ngi" → ["/etc", "ngi"], "/etc/" → ["/etc", ""]. */
export function splitForSuggest(input: string, cwd: string, home: string): { dir: string; prefix: string } {
  const t = input.trim();
  if (!t || t.endsWith("/") || t === "~") return { dir: resolvePath(t || ".", cwd, home), prefix: "" };
  const i = t.lastIndexOf("/");
  if (i < 0) return { dir: cwd, prefix: t };
  return { dir: resolvePath(t.slice(0, i + 1) || "/", cwd, home), prefix: t.slice(i + 1) };
}

/** Folder names that start with the prefix (case-insensitive), exact case first, then by name. */
export function suggest(names: string[], prefix: string, limit = 12): string[] {
  const p = prefix.toLowerCase();
  return names
    .filter((n) => n.toLowerCase().startsWith(p) && (p.startsWith(".") || !n.startsWith(".")))
    .sort((a, b) => Number(!a.startsWith(prefix)) - Number(!b.startsWith(prefix)) || a.localeCompare(b))
    .slice(0, limit);
}

/** Keeps the last few folders gone to (newest first, no repeats). */
export function remember(list: string[], path: string, max = 8): string[] {
  return [path, ...list.filter((x) => x !== path)].slice(0, max);
}
