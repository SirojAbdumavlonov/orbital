import { BookOpen, ExternalLink, Keyboard, LifeBuoy, MessageSquare, Shield, Sparkles, Wrench } from "lucide-react";
import { ReactNode } from "react";
import { isMac, isWindows, KEYS } from "../lib/platform";
import { openRepoUrl, REPO_URL } from "../lib/links";
import { useStore } from "../store";

// The Help page: short answers for getting started, everyday tasks, the
// problems people actually hit, keyboard shortcuts and where data lives.

function Section({ icon, title, children }: { icon: ReactNode; title: string; children: ReactNode }) {
  return (
    <section className="help-section">
      <h3>{icon} {title}</h3>
      {children}
    </section>
  );
}

function Q({ q, children }: { q: string; children: ReactNode }) {
  return (
    <details className="help-q">
      <summary>{q}</summary>
      <div>{children}</div>
    </details>
  );
}

export function HelpView() {
  const { setEditing, setImportingSsh, setGlobalView } = useStore();
  const dataDir = isWindows ? "%LOCALAPPDATA%\\Orbital" : isMac ? "~/Library/Application Support/.orbital" : "~/.local/share/orbital";
  return (
    <div className="view">
      <div className="scroll">
        <div className="settings help">
          <div className="help-actions">
            <button className="btn" onClick={() => setGlobalView("feedback")}><MessageSquare size={13} /> Send feedback</button>
            <button className="btn" onClick={() => setGlobalView("changes")}><Sparkles size={13} /> What's new</button>
            <button className="btn" onClick={() => openRepoUrl(`${REPO_URL}/issues`)}><ExternalLink size={13} /> Known issues on GitHub</button>
          </div>

          <Section icon={<BookOpen size={15} />} title="Getting started">
            <ol className="help-steps">
              <li>
                <b>Add a server</b> ({KEYS.newServer}): its address, your user name, and a password or SSH key. Servers you
                already use in a terminal can be brought in from <span className="mono">~/.ssh/config</span>.
                <div className="help-inline">
                  <button className="btn" onClick={() => setEditing("new")}>Add server</button>
                  <button className="btn" onClick={() => setImportingSsh(true)}>Import from ~/.ssh/config</button>
                </div>
              </li>
              <li><b>Connect</b> by clicking it in the sidebar. The first time, Orbital shows the server's fingerprint: trust it only if it's the server you expect.</li>
              <li><b>Allow sudo</b> if you manage the system: in the server's settings, Administrator access → "sudo with password" or "sudo without password". Orbital then retries with sudo whenever the server refuses your user.</li>
              <li><b>Add tabs</b> with <b>+</b> next to the server's tabs: Orbital lists the software it found (Docker, nginx, databases…) and offers to install the rest.</li>
            </ol>
          </Section>

          <Section icon={<Wrench size={15} />} title="Common tasks">
            <Q q="Edit a config file safely">
              Open it in <b>Files</b> and save. Orbital keeps a backup, checks the file (nginx -t, sshd -t, …) and puts the old one back if the check
              fails. <b>Versions</b> in the editor lists earlier backups to open or restore.
            </Q>
            <Q q="Put a website online with HTTPS">
              The server's <b>Nginx</b> tab → <b>New website</b>: the domain, what it serves (a folder or an app's port), then DNS and the
              certificate. <b>Test</b> checks it on the server and from this computer; <b>Delete website…</b> removes it with a backup.
            </Q>
            <Q q="Open a web interface that only listens on the server (Portainer, Grafana…)">
              <b>Port forwarding</b> (from <b>+</b>, or a container's port in Docker) makes it available on this computer at
              <span className="mono"> localhost:PORT</span>, through the SSH connection. Nothing is opened to the internet.
            </Q>
            <Q q="Move a container to another server">
              Docker → the container → <b>Move to server…</b>. Check the preview: ports, arguments and environment can be adjusted for the new server.
            </Q>
            <Q q="Run the same command on several servers">
              Save it as a <b>Snippet</b> (sidebar), with <span className="mono">{"{{parameters}}"}</span> if it needs values, and run it on the servers you choose.
            </Q>
            <Q q="Get told when a website goes down">
              <b>Uptime</b> (sidebar) checks websites and ports from this computer and shows a notification when one stops answering. Checks run while Orbital is open.
            </Q>
          </Section>

          <Section icon={<LifeBuoy size={15} />} title="When something goes wrong">
            <Q q={`"Not connected" or "the connection to this server was lost"`}>
              The connection dropped: the computer slept, the network changed or the server restarted. Click <b>Reconnect</b>; your tab, folder and selections are kept.
            </Q>
            <Q q={`"Permission denied"`}>
              Your user isn't allowed to do that. Allow sudo in the server's settings (Administrator access), and Orbital retries with sudo. Commands you run
              yourself (snippets) offer <b>Run again with sudo</b> instead of retrying on their own.
            </Q>
            <Q q="A website or port works on the server but not from the internet">
              A firewall is in the way. Besides ufw on the server (the <b>Firewall</b> tab), most cloud providers have their own: Oracle Cloud's security
              list, AWS security groups, Hetzner's firewall… Allow the port there too. A website's <b>Test</b> tells you which side fails.
            </Q>
            <Q q={`"The host key has changed"`}>
              The server presents a different identity than before. That's expected after reinstalling it; otherwise someone may be in between. Don't
              connect until you know why. Saved keys are in <b>Security</b>.
            </Q>
            <Q q="The HTTPS certificate wasn't issued">
              The domain must point at this server (the dialog checks it), and ports 80 and 443 must be open to the internet, including in the cloud firewall.
              A new DNS record can take a few minutes to be seen everywhere.
            </Q>
            <p className="faint help-more">
              Still stuck? <button className="wn-link" onClick={() => setGlobalView("feedback")}>Tell us what happened</button>: the error message and what you clicked help the most.
            </p>
          </Section>

          <Section icon={<Keyboard size={15} />} title="Keyboard shortcuts">
            <table className="help-keys">
              <tbody>
                <tr><td><kbd>{KEYS.commandMenu}</kbd></td><td>Command menu: jump to any server, tab or page</td></tr>
                <tr><td><kbd>{KEYS.newServer}</kbd></td><td>Add a server</td></tr>
                <tr><td><kbd>{KEYS.back}</kbd> <kbd>{KEYS.forward}</kbd></td><td>Back and forward between places</td></tr>
                <tr><td><kbd>{KEYS.sidebar}</kbd></td><td>Show or hide the sidebar</td></tr>
                <tr><td><kbd>→</kbd></td><td>In the terminal, at the end of a line: accept the suggestion</td></tr>
              </tbody>
            </table>
          </Section>

          <Section icon={<Shield size={15} />} title="Your data">
            <ul className="help-list">
              <li>Servers, trusted host keys and the activity log stay on this computer, in <span className="mono">{dataDir}</span>.</li>
              <li>Saved passwords and key passphrases are encrypted and can be read only on this computer.</li>
              <li>Every change Orbital makes on a server is written to the <b>Activity log</b>, chained so that later edits to it can be detected.</li>
              <li>Anonymous usage statistics are sent only if you said yes (Settings → Privacy). Feedback is sent only when you click Send.</li>
            </ul>
          </Section>
        </div>
      </div>
    </div>
  );
}
