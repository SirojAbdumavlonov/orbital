// asciinema v2 recordings (.cast): a JSON header line, then [time, kind, data] lines.

export interface CastEvent { t: number; kind: "o" | "r" | string; data: string }
export interface Cast { width: number; height: number; title: string; events: CastEvent[]; length: number }

/**
 * Parse a recording. Pauses longer than `idleLimit` seconds are shortened to
 * it (like `asciinema play -i`), so playback doesn't sit still while someone
 * read or went for coffee. Broken lines are skipped.
 */
export function parseCast(text: string, idleLimit = Infinity): Cast {
  const lines = text.split("\n").filter((l) => l.trim());
  let header: { width?: number; height?: number; title?: string } = {};
  try {
    header = JSON.parse(lines[0] ?? "{}");
  } catch {
    /* not a recording: no events */
  }
  const events: CastEvent[] = [];
  let prev = 0;
  let shift = 0;
  for (const l of lines.slice(1)) {
    let e: unknown;
    try {
      e = JSON.parse(l);
    } catch {
      continue;
    }
    if (!Array.isArray(e) || typeof e[0] !== "number" || typeof e[2] !== "string") continue;
    const gap = e[0] - prev;
    if (gap > idleLimit) shift += gap - idleLimit;
    prev = e[0];
    events.push({ t: Math.max(0, e[0] - shift), kind: String(e[1]), data: e[2] });
  }
  return {
    width: header.width ?? 80,
    height: header.height ?? 24,
    title: header.title ?? "",
    events,
    length: events.length ? events[events.length - 1].t : 0,
  };
}
