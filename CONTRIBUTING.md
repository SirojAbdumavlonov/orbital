# Contributing

1. `npm install`, then `npm run tauri dev`.
2. Engine changes go in `crates/outpost-core` with tests. Anything that touches a server needs a case in `tests/live_ssh.rs`.
3. Any new state-changing operation must go through `Engine` and record an audit event.
4. Any value interpolated into a remote command must use `shell::quote` (plus a validator for identifiers).
5. `cargo clippy --all-targets -- -D warnings`, `cargo test`, `npm run typecheck` and `npm test` must pass.

## Renaming the product

"Outpost" is a working name. To rename, change these places:

| What | Where |
|---|---|
| App name and window title | `src-tauri/tauri.conf.json` → `productName`, `app.windows[0].title` |
| Bundle identifier | `src-tauri/tauri.conf.json` → `identifier` (`app.outpost.desktop`) |
| Keychain service name | `src-tauri/src/lib.rs` → `KEYCHAIN_SERVICE`. Changing it orphans saved passwords, so migrate them or ask users to re-enter them. |
| App data folder | Derived from the identifier (`~/Library/Application Support/<identifier>`) |
| Crate and package names | `Cargo.toml` files (`outpost`, `outpost-core`, `outpost_lib`), `package.json` |
| Backup and temp file suffixes | `crates/outpost-core/src/config_edit.rs` (`.outpost-bak-`, `/tmp/.outpost-`) |
| Logo and icons | `design/logo.svg`, then `npx tauri icon design/logo-1024.png -o src-tauri/icons` |
| Copy | `README.md`, `src/components/Welcome.tsx`, `index.html` |
