# Security model

Outpost holds the keys to production servers, so the defaults are conservative.

## What we protect against

| Threat | Mitigation |
|---|---|
| Man-in-the-middle on first or later connection | Host keys are pinned per host and port (TOFU with explicit fingerprint confirmation). A changed key is a hard failure, and the UI offers no "connect anyway". Keys from `~/.ssh/known_hosts` are honoured. |
| Secrets at rest | Passwords, key passphrases and sudo passwords live only in the macOS Keychain. The SQLite database holds no secrets and is created `0600`. In memory, secrets are `Zeroizing<String>`. |
| Secrets in transit or in logs | The sudo password is written to the SSH channel's stdin, never into argv, so it doesn't show up in `ps`, shell history or the audit log. Tests assert the password never appears in any audit record. |
| Command injection through file names, unit names or container names | All interpolations are POSIX single-quoted. Identifiers are additionally validated against strict regexes, and option-like names (`-H`, `--privileged`) are rejected. |
| Breaking a server with a bad config | Validators run before or after install, with automatic rollback. Pre-flight checks run before service restarts and reloads. A backup is kept for every config save. |
| Locking yourself out | `sshd -t` runs before any sshd reload or restart, and before saving sshd_config. Stopping ssh requires typing the service name. |
| Repudiation, or someone editing the log | The audit log is append-only (DB triggers) and SHA-256 hash-chained. "Verify" recomputes the chain. |
| Accidental destructive actions | Confirmation dialogs for stop, disable, delete and remove. Typed confirmation for recursive folder deletes and container removal. `/` can't be deleted. |
| Runaway output | Command output capture is capped at 32 MB per stream. Editor files are capped at 10 MB. All commands have timeouts. |

## Out of scope (for now)

- A compromised Mac: anything with your user's privileges can read your Keychain items after you approve access.
- Terminal sessions aren't recorded keystroke by keystroke. Opening and closing them is audited.
- The audit chain proves integrity, not authenticity: someone with write access to the DB file could rebuild the whole chain. A signed or remote-anchored log (for example shipping hashes to a server) is on the roadmap.

## App hardening

- Tauri capabilities allow only `core:default` and the open, save and message dialogs. No shell, fs or http plugins are exposed to the webview.
- Strict CSP: no remote scripts, and Monaco is self-hosted.
- Hardened runtime with only the network-client and user-selected-files entitlements. Distributed as a Developer-ID-signed, notarized app outside the App Store, like Cyberduck's direct download, because the App Sandbox would block `~/.ssh` keys and the ssh-agent socket.

## Reporting a vulnerability

Please email security@outpost.app (placeholder until the domain is set up) rather than opening a public issue.
