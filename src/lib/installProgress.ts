// How far an install from the "+" window has got, 0–100, from the lines the
// install script prints (orbital_core::install::script):
//   "@@progress N"            between phases (start 2, packages 10, setup 92, service 96)
//   "dlstatus:…:PCT:…"        apt downloading  → 10–50
//   "pmstatus:…:PCT:…"        apt installing   → 50–90
//   "(3/12): name…"           dnf/yum downloading → 10–50
//   "Installing : pkg 3/12", "[ 3/12] Installing…"  dnf/yum installing → 50–90
// It never goes back. The markers and apt's status lines aren't shown as output.

const clamp = (n: number) => Math.max(0, Math.min(100, n));

export function installPercent(lines: string[]): number {
  let p = 0;
  for (const raw of lines) {
    const l = raw.trim();
    let v: number | null = null;
    let m: RegExpExecArray | null;
    if ((m = /^@@progress (\d+(?:\.\d+)?)$/.exec(l))) v = Number(m[1]);
    else if ((m = /^dlstatus:[^:]*:(\d+(?:\.\d+)?):/.exec(l))) v = 10 + Number(m[1]) * 0.4;
    else if ((m = /^pmstatus:[^:]*:(\d+(?:\.\d+)?):/.exec(l))) v = 50 + Number(m[1]) * 0.4;
    else if ((m = /^\((\d+)\/(\d+)\):/.exec(l))) v = 10 + (40 * Number(m[1])) / Math.max(1, Number(m[2]));
    else if ((m = /^\[\s*(\d+)\/(\d+)\]/.exec(l)) || (m = /^(?:Installing|Upgrading|Running scriptlet|Verifying|Preparing|Cleanup)\b.*\s(\d+)\/(\d+)$/.exec(l))) {
      v = 50 + (40 * Number(m[1])) / Math.max(1, Number(m[2]));
    }
    if (v !== null && Number.isFinite(v)) p = Math.max(p, clamp(v));
  }
  return Math.round(p);
}

/** Lines meant for the progress ring, not for people. */
export const isProgressLine = (l: string) => /^(@@progress |dlstatus:|pmstatus:|dpkg-status:|pmconffile:)/.test(l.trim());
