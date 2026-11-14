export function bytes(n: number | null | undefined, digits = 1): string {
  if (n == null || !isFinite(n)) return "—";
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  let i = 0;
  let v = Math.abs(n);
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${i === 0 ? v.toFixed(0) : v.toFixed(digits)} ${units[i]}`;
}

export function rate(n: number | null | undefined): string {
  return n == null ? "—" : `${bytes(n)}/s`;
}

export function duration(secs: number): string {
  const d = Math.floor(secs / 86400);
  const h = Math.floor((secs % 86400) / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}

export function percent(n: number | null | undefined, digits = 0): string {
  return n == null ? "—" : `${n.toFixed(digits)}%`;
}

export function relativeTime(unixSecs: number | null | undefined, now = Date.now()): string {
  if (!unixSecs) return "—";
  const diff = Math.round(now / 1000 - unixSecs);
  if (diff < 60) return "just now";
  if (diff < 3600) return `${Math.floor(diff / 60)} min ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)} h ago`;
  if (diff < 86400 * 30) return `${Math.floor(diff / 86400)} d ago`;
  return new Date(unixSecs * 1000).toLocaleDateString();
}

export function joinPath(dir: string, name: string): string {
  return dir.endsWith("/") ? dir + name : `${dir}/${name}`;
}

export function parentPath(path: string): string {
  const t = path.replace(/\/+$/, "");
  const i = t.lastIndexOf("/");
  return i <= 0 ? "/" : t.slice(0, i);
}

export function baseName(path: string): string {
  const t = path.replace(/\/+$/, "");
  return t.slice(t.lastIndexOf("/") + 1) || "/";
}

export function breadcrumbs(path: string): { name: string; path: string }[] {
  const parts = path.split("/").filter(Boolean);
  const out = [{ name: "/", path: "/" }];
  let acc = "";
  for (const p of parts) {
    acc += "/" + p;
    out.push({ name: p, path: acc });
  }
  return out;
}

/** "rwxr-xr-x" -> 0o755 and back helpers for the permissions dialog. */
export function octal(mode: number): string {
  return (mode & 0o7777).toString(8).padStart(3, "0");
}
