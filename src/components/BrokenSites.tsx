import { FileText, Power } from "lucide-react";
import { useState } from "react";
import { api, BrokenSite } from "../lib/api";
import { runJob } from "../lib/jobs";
import { useStore } from "../store";

// When nginx's configuration is broken by an enabled site that needs something
// that's gone (a deleted certificate, a removed file), say which and offer to
// turn that site off by its sites-enabled link. The file itself is kept, and
// the link is put back if nginx still fails. Files that aren't links (conf.d)
// are opened in the editor instead.

export function brokenReason(b: BrokenSite): string {
  if (b.dangling) return "its file was removed, but the link that enables it is still there";
  const certs = [...new Set(b.missing.map((c) => c.replace(/^\/etc\/letsencrypt\/live\//, "").split("/")[0]))];
  return `it uses the certificate${certs.length > 1 ? "s" : ""} ${certs.join(", ")}, which ${certs.length > 1 ? "were" : "was"} deleted`;
}

export function BrokenSitesFix({ sid, sites, onFixed }: { sid: string; sites: BrokenSite[]; onFixed: () => void }) {
  const [busy, setBusy] = useState<string | null>(null);
  const openFile = useStore((x) => x.openFile);
  if (!sites.length) return null;
  async function off(b: BrokenSite) {
    setBusy(b.file);
    try {
      await runJob((p) => api.disableSite(sid, b.file, p), { key: `site-off:${sid}`, title: `Turning ${b.file.split("/").pop()} off`, sid, done: () => `${b.file.split("/").pop()} is off; nginx works again` });
      onFixed();
    } catch {
      // The job reports it (and the link is back if nginx still failed).
    } finally {
      setBusy(null);
    }
  }
  return (
    <div className="broken-sites">
      <b>What breaks nginx</b>
      {sites.map((b) => (
        <div key={b.file} className="broken-site">
          <div>
            <span className="mono">{b.file}</span>: {brokenReason(b)}.
            {b.link && <div className="faint">Turning it off removes only its link in sites-enabled; {b.stock ? "nginx's default file" : "the site's file"} stays as it is{b.dangling ? "" : ", so you can turn it back on or create the site again later"}. If nginx still fails, the link is put back.</div>}
          </div>
          {b.link
            ? <button className="btn primary" disabled={!!busy} onClick={() => off(b)}><Power size={13} /> {busy === b.file ? "Turning it off…" : "Turn this site off"}</button>
            : <button className="btn" onClick={() => void openFile(sid, b.file)}><FileText size={13} /> Open in the editor</button>}
        </div>
      ))}
    </div>
  );
}
