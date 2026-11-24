/** Orbital's GitHub repository: the only remote web address the app opens (the opener permission is scoped to it and to http://localhost). */
export const REPO_URL = "https://github.com/SirojAbdumavlonov/orbital";

/** Opens a page of Orbital's repository in the browser; anything else is ignored. */
export async function openRepoUrl(url: string) {
  if (!url.startsWith(REPO_URL + "/") && url !== REPO_URL) return;
  try {
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(url);
  } catch {
    window.open(url, "_blank", "noopener");
  }
}

/** Opens http://localhost:<port>/ (a forwarded app) in the browser; the opener allows nothing else local. */
export async function openLocalPort(port: number) {
  if (!Number.isInteger(port) || port < 1 || port > 65535) return;
  const url = `http://localhost:${port}/`;
  try {
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(url);
  } catch {
    window.open(url, "_blank", "noopener");
  }
}

/**
 * A new GitHub issue with the feedback filled in, for builds without a
 * feedback server. The contact is left out: issues are public.
 */
export function issueUrl(kind: string, about: string, message: string, version: string, os: string): string {
  const title = `${kind === "problem" ? "Problem" : kind === "idea" ? "Idea" : "Feedback"}${about ? `: ${about}` : ""}`;
  const body = `${message.trim()}\n\n---\nOrbital ${version} · ${os}`;
  const q = new URLSearchParams({ title, body });
  return `${REPO_URL}/issues/new?${q.toString()}`;
}
