# Security model

Orbital holds the keys to production servers, so the defaults are conservative.

## What we protect against

| Threat | Mitigation |
|---|---|
| Man-in-the-middle on first or later connection | Host keys are pinned per host and port (TOFU with explicit fingerprint confirmation). A changed key is a hard failure, and the UI offers no "connect anyway". Keys from `~/.ssh/known_hosts` are honoured. |
| A compromised or hostile jump host | Each hop on a ProxyJump route is its own SSH session, tunnelled through the previous one, with its host key checked against the pin for its own host and port. A jump host carries only ciphertext for the servers behind it and is never sent their passwords: a typed secret is used for the one profile it was typed for. No agent forwarding. A fingerprint confirmed in the trust dialog is pinned only for the endpoint the dialog named. A missing or looping jump host fails the connection; Orbital never falls back to connecting directly. |
| Someone using your unlocked Mac | Optional Touch ID gate (Security → "Require Touch ID before using saved credentials"). With it on, a saved password, key passphrase or sudo password is released only after Touch ID or the login password, checked for every connection, and locked again once authentication is over. Turning the gate on or off needs the same check. Cancelled and failed checks are audited. |
| Secrets at rest | Passwords, key passphrases and sudo passwords are kept in Orbital's own vault: one file encrypted with ChaCha20-Poly1305, whose key comes from a random per-install key file and this Mac's hardware UUID, so the data folder copied to another computer is unreadable. The folder (`~/Library/Application Support/.orbital`, hidden in Finder) is `0700`, the files `0600`. The SQLite database holds no secrets. In memory, secrets are `Zeroizing<String>`. Orbital doesn't use the Keychain, so it never shows Keychain access prompts. |
| Secrets in transit or in logs | The sudo password is written to the SSH channel's stdin, never into argv, so it doesn't show up in `ps`, shell history or the audit log. Tests assert the password never appears in any audit record. |
| Command injection through file names, unit names or container names | All interpolations are POSIX single-quoted. Identifiers are additionally validated against strict regexes, and option-like names (`-H`, `--privileged`) are rejected. |
| Copying between servers exposing one to the other | A server-to-server copy reads from one SSH session and writes to the other inside Orbital. The servers never connect to each other, and no credential or key of one is sent to the other. The copy is recorded in the audit log of the destination. |
| Software detection running something unexpected | The detection script is built only from a catalogue compiled into the app. No file name, user input or server output is interpolated into it, and it only reads (`command -v`, `systemctl show`, `pgrep`/`ps`, file-existence tests). |
| Locking yourself out with the firewall or fail2ban | Before any ufw change (add, change, delete, turn on) the engine checks that the resulting rules still let the current SSH connection in, using the client address and server port from `$SSH_CONNECTION`. If not, the change isn't sent unless the user types a confirmation. Banning the connection's own address in fail2ban is refused. Rules are deleted by number only after the number is checked against the rule text the user saw. |
| Command injection through firewall rules, domains, jail names or IPs | Rules are built from a structured form (action, port list, protocol, address or network, app profile, comment), with every field validated (ports and ranges, IP and CIDR parsing, app-name and jail-name allow-lists) and quoted. Domain names, e-mail addresses and certificate names are checked against strict patterns; wildcards are refused. |
| Breaking HTTPS by deleting a certificate in use | Deleting a certificate first searches the nginx and Apache configs for its path and, if found, needs a typed confirmation. Private keys are never opened, downloaded or shown, only their paths. |
| The sudo password in a root terminal | A root terminal runs `sudo` with a marker prompt; Orbital hides the marker and types the saved sudo password once, which sudo reads with echo off. The password never goes through the UI, the screen or the activity log (the log records "terminal.open … (as root)"). A wrong password is not retried: sudo's normal prompt appears for the user. |
| A new user's password leaking | It is sent to `chpasswd` over the SSH channel's stdin, after the sudo password line, never on a command line, so it doesn't appear in `ps`, shell history or the activity log. A test checks the log. Users can be created with SSH keys only, in which case the password is locked. |
| Locking yourself out through user management | Orbital refuses to delete or lock the account the connection uses, or root. Removing your own sudo group or giving yourself a no-login shell needs a typed confirmation. |
| Database dumps left on the server | A backup is written to a private (`umask 077`) temp file, handed to the login user only for the download, and deleted afterwards whether the download worked or not. A failed dump leaves nothing behind. Redis's `requirepass` is read on the server into `REDISCLI_AUTH`; it is never sent to the Mac. |
| Tunnels exposing a server's port | A tunnel listens on 127.0.0.1 only, so other machines on your network can't use it, and the traffic travels inside the SSH connection. Nothing is opened in the server's firewall. Tunnels close with the connection and are audited. |
| A deploy destroying work on the server | Deploys only fast-forward (`git merge --ff-only`); they never reset, force or discard. If tracked files were changed on the server the deploy is refused. Rollback checks out an existing commit, and every step's output is kept with the deploy record. |
| FTP, WebDAV and S3 credentials | Kept in the vault like SSH passwords and read only through the Touch ID gate when it is on. TLS (FTPS, WebDAV over HTTPS, S3) checks certificates against the public certificate authorities; FTPS never falls back to plain FTP. S3 requests are signed (Signature Version 4), so the secret key itself is never sent. |
| Breaking a server with a bad config | Validators run before or after install, with automatic rollback. Pre-flight checks run before service restarts and reloads. A backup is kept for every config save. |
| Locking yourself out | `sshd -t` runs before any sshd reload or restart, and before saving sshd_config. Stopping ssh requires typing the service name. |
| Repudiation, or someone editing the log | The audit log is append-only (DB triggers) and SHA-256 hash-chained. "Verify" recomputes the chain. |
| Accidental destructive actions | Confirmation dialogs for stop, disable, delete and remove. Typed confirmation for recursive folder deletes and container removal. `/` can't be deleted. |
| Runaway output | Command output capture is capped at 32 MB per stream. Editor files are capped at 10 MB. All commands have timeouts. |

## Out of scope (for now)

- A compromised Mac: anything running as your user can read the vault's key file and hardware ID, and so the saved credentials. The Keychain would ask before handing items to another app, which the vault does not; that is the price of never being asked. Turn on the Touch ID gate if other people use this Mac.
- The Touch ID gate is enforced by Orbital. It stops a person at your keyboard from connecting with saved credentials; it does not stop malware running as you, which can read the vault files directly.
- The first connection through a jump host is trust-on-first-use like any other: until the target's key is pinned, the jump host could present a server of its own. Compare the fingerprint in the dialog.
- Requesting a certificate with the nginx or Apache method lets certbot edit those configs itself. certbot keeps its own backups (`certbot rollback`), but Orbital's safe save isn't involved.
- Plain FTP and WebDAV over HTTP send the password unencrypted, as those protocols do. The server dialog says so; use FTPS, HTTPS or SFTP when the server offers them. Servers with self-signed certificates are refused for now (no way to trust one yet).
- Terminal sessions aren't recorded keystroke by keystroke. Opening and closing them is audited.
- The audit chain proves integrity, not authenticity: someone with write access to the DB file could rebuild the whole chain. A signed or remote-anchored log (for example shipping hashes to a server) is on the roadmap.

## App hardening

- Tauri capabilities allow only `core:default`, window dragging from the top bars, and the open, save and message dialogs. No shell, fs or http plugins are exposed to the webview.
- Strict CSP: no remote scripts, and Monaco is self-hosted.
- Updates are installed only when their signature matches the updater public key built into the app (minisign, `plugins.updater.pubkey` in `tauri.conf.json`), and only after the user clicks Update. They are downloaded over HTTPS from the GitHub release. The private key is a CI secret and a backup kept by the maintainer, never in the repository; whoever holds it can ship an update to every installed copy.
- Hardened runtime with only the network-client and user-selected-files entitlements. Distributed as a Developer-ID-signed, notarized app outside the App Store, like Cyberduck's direct download, because the App Sandbox would block `~/.ssh` keys and the ssh-agent socket.

## Reporting a vulnerability

Please report it privately through **Report a vulnerability** on the repository's Security tab (GitHub private vulnerability reporting), not in a public issue. We aim to reply within a week.
