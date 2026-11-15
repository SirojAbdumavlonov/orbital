# Contributing

1. `npm install`, then `npm run tauri dev`.
2. Engine changes go in `crates/orbital-core` with tests. Anything that touches a server needs a case in `tests/live_ssh.rs`.
3. Any new state-changing operation must go through `Engine` and record an audit event.
4. Any value interpolated into a remote command must use `shell::quote` (plus a validator for identifiers).
5. `cargo fmt --all`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `npm run typecheck` and `npm test` must pass.
6. Anything that can lock a user out of their server (firewall, fail2ban, sshd) needs a pre-flight check in the engine, not only a warning in the UI.

## Releasing

1. Update the version in `src-tauri/tauri.conf.json`, `package.json` and the workspace `Cargo.toml`, and move the "Unreleased" notes in `CHANGELOG.md` under the new version.
2. Commit, then tag and push: `git tag v1.0.0 && git push origin v1.0.0`.
3. The Release workflow checks the tag matches the app version, builds a universal `.dmg` and attaches it to a draft release. Review the draft and publish it.
4. For a signed and notarized build, add the `APPLE_*` secrets listed at the top of `.github/workflows/release.yml` first.

## Renaming the product

"Orbital" is a working name. To rename, change these places:

| What | Where |
|---|---|
| App name and window title | `src-tauri/tauri.conf.json` → `productName`, `app.windows[0].title` |
| Bundle identifier | `src-tauri/tauri.conf.json` → `identifier` (`app.orbital.desktop`) |
| Data folder | `src-tauri/src/lib.rs` → `DATA_DIR` (`.orbital` under Application Support). Add the previous folder to `OLD_HOMES` so `move_old_data` moves the database over. The vault is keyed to the machine, not the name. |
| Crate and package names | `Cargo.toml` files (`orbital`, `orbital-core`, `orbital_lib`), `package.json` |
| Backup and temp file suffixes | `crates/orbital-core/src/config_edit.rs` (`.orbital-bak-`, `/tmp/.orbital-`) |
| Logo and icons | `design/logo.svg`, then `npx tauri icon design/logo-1024.png -o src-tauri/icons` |
| Copy | `README.md`, `src/components/Welcome.tsx`, `index.html` |
