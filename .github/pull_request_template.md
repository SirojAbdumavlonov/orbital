## What and why

<!-- What this changes, and the problem it solves. Link the issue if there is one. -->

## How it was tested

- [ ] `cargo clippy -p orbital-core --all-targets -- -D warnings` and `cargo test -p orbital-core`
- [ ] `npm run typecheck`, `npm test`, `npm run build`
- [ ] Live SSH tests, if this touches a server (`tests/live_ssh.rs`)
- [ ] Checked in the app or `npm run demo` (screenshots for UI changes)

## Safety checklist

- [ ] Every state change goes through `Engine` and is audited
- [ ] Every value in a remote command goes through `shell::quote` (and a validator for identifiers)
- [ ] No secrets in SQLite, logs or the audit trail
