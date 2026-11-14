# Architecture

```
┌──────────────────────── Outpost.app ────────────────────────┐
│  React UI (WKWebView)                                        │
│  Files · Editor · Services · Docker · Monitor · Terminal ·   │
│  Activity                                                    │
│        │ invoke() / Channel (typed, src/lib/api.ts)          │
│  ──────┼──────────────────────────────────────────────────── │
│  src-tauri  (thin command layer, no logic)                   │
│        │                                                     │
│  outpost-core::Engine  ◄── the only entry point; audits      │
│   ├── ssh        connect, host-key check, auth, exec, sudo   │
│   ├── sftp       list/read/write/upload/download/chmod       │
│   ├── config_edit safe save: stage → validate → backup →     │
│   │               install → validate → rollback              │
│   ├── services   systemd list/show/journal + guarded actions │
│   ├── docker     docker CLI (JSON formats) + sudo fallback   │
│   ├── metrics    one-shot /proc script, parsed locally       │
│   ├── terminal   PTY channels streamed to xterm.js           │
│   ├── audit      append-only, SHA-256 hash chain             │
│   ├── known_hosts TOFU pinning + ~/.ssh/known_hosts          │
│   ├── profiles   bookmarks (no secrets)                      │
│   └── secrets    macOS Keychain                              │
│        │                         │                           │
│     SQLite (0600)            Keychain                        │
└────────┼─────────────────────────────────────────────────────┘
         │  one SSH connection per server (multiplexed channels)
         ▼
   sshd ─ sftp-server · sh (exec) · PTY shell
```

## Principles

1. **Agentless.** Everything uses what any Linux server already has: sshd, the SFTP subsystem, POSIX `sh`, `/proc`, `systemctl`, `journalctl` and the `docker` CLI. Nothing is installed remotely.
2. **One connection, many channels.** SFTP, command execution, metrics polling and every terminal tab are channels on the same SSH session. You authenticate once.
3. **Every mutation goes through `Engine` and is audited.** The Tauri layer can't reach `SshSession` directly. Reads (listing, viewing, metrics) aren't logged; changes always are, including failed and blocked attempts.
4. **No string-built shell injection.** Every interpolated value passes through `shell::quote`. Unit names and container ids must also match a strict allow-list before they reach a command line.
5. **Core without UI.** `outpost-core` has no Tauri dependency, so it is tested headless against a real sshd in CI.

## Key flows

**Connecting.** `russh` calls `check_server_key`. The key is looked up first in Outpost's pinned keys, then in `~/.ssh/known_hosts`. An unknown key fails the connection with `UnknownHostKey { fingerprint }`, and the UI shows the trust dialog. Reconnecting with `accept_fingerprint` pins exactly that fingerprint and nothing else. A mismatch is always refused, even if a fingerprint was "accepted". Authentication then follows the profile: password (falling back to keyboard-interactive), a key file or ssh-agent identities.

**Privileged commands.** `exec_privileged(cmd)` wraps the command as `sudo -n -- sh -c '<cmd>'` (passwordless) or `sudo -k -S -p '' -- sh -c '<cmd>'` with the password written to the channel's stdin. `-k` forces sudo to read the password every time, so it can never leak into the command's own stdin.

**Safe save.** See `config_edit.rs`. Validators are picked by path. *Pre-install* validators (`visudo -cf`, `haproxy -c -f`) check the temp file before the real file is touched. *Post-install* validators (`nginx -t`, `sshd -t`) check the live tree and trigger a rollback from the backup if they fail. The write is `cat tmp > file` rather than a rename, so ownership, mode, ACLs and the inode (which matters for Docker bind mounts) are preserved.

**Metrics.** One `exec` per tick runs `SNAPSHOT_SCRIPT`, which prints tagged sections. `parse_snapshot` turns them into a struct, and `derive` computes CPU percentages and network rates from the previous sample. The UI polls every 2 s for the visible server and every 10 s for background ones.

**Audit chain.** `hash_n = SHA-256(hash_{n-1} ‖ canonical_json(row_n))`, with the genesis hash being 64 zeros. Triggers reject `UPDATE`/`DELETE`. `verify()` walks the chain and reports the first row that doesn't match.
