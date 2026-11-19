/** Helpers for log panes: how bad a line looks, keeping the newest lines, filtering. */

export type Level = "error" | "warn" | null;

/** How bad a line looks, for colouring and "errors only". */
export function level(t: string): Level {
  if (/\b(error|err|fatal|crit(ical)?|emerg|alert|panic|exception|failed|traceback)\b|\s5\d\d\s/i.test(t)) return "error";
  if (/\b(warn(ing)?|deprecated)\b|\s4\d\d\s/i.test(t)) return "warn";
  return null;
}

/** Keep the newest `max` lines, so a chatty source doesn't grow the page forever. */
export function keepTail(text: string, max = 2000): string {
  let n = 0;
  for (let i = text.length - 1; i >= 0; i--) if (text[i] === "\n" && ++n > max) return text.slice(i + 1);
  return text;
}

/** The lines to show: containing `query` (any case) and, with `problems`, only errors and warnings. */
export function filterLines(text: string, query: string, problems: boolean): { text: string; level: Level }[] {
  const q = query.trim().toLowerCase();
  const out: { text: string; level: Level }[] = [];
  for (const line of text.split("\n")) {
    if (!line && out.length === 0) continue;
    if (q && !line.toLowerCase().includes(q)) continue;
    const lv = level(line);
    if (problems && !lv) continue;
    out.push({ text: line, level: lv });
  }
  while (out.length && !out[out.length - 1].text) out.pop();
  return out;
}
