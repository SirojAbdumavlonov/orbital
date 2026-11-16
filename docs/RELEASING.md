# Releasing Orbital

Installed copies of Orbital update themselves: they read `latest.json` from the
newest published GitHub release, and when its version is higher they offer the
update (a toast, and Settings → Updates). The download is a signed
`Orbital.app.tar.gz`; the app installs it only if the signature matches the
public key in `src-tauri/tauri.conf.json` (`plugins.updater.pubkey`), then
asks to restart. Data in `~/Library/Application Support/.orbital` is untouched,
and database migrations run on the next start.

## Versions

[Semantic Versioning](https://semver.org/), one number for the whole app
(`Cargo.toml`, `src-tauri/tauri.conf.json`, `package.json`):

| Release | Command | Example |
|---|---|---|
| Fixes only | `npm run release -- patch` | 1.0.0 → 1.0.1 |
| New features | `npm run release -- minor` | 1.0.1 → 1.1.0 |
| Users must adapt | `npm run release -- major` | 1.1.0 → 2.0.0 |
| A test build | `npm run release -- beta` | 1.1.0 → 1.1.1-beta.1 |

Group changes into a release every week or two rather than one per change.
Betas are published as GitHub prereleases, which installed copies are never
offered: send the `.dmg` to whoever tests them.

## One-time setup

1. **The updater key.** It was made with
   `npx tauri signer generate -w ~/.tauri/orbital-updater.key` and has no
   password. Back up `~/.tauri/orbital-updater.key` somewhere safe (a password
   manager). If it is lost, installed copies can't be updated any more and
   everyone has to download the app again by hand. Never commit it.
2. **GitHub.** Create the repository and push. Then, under Settings → Secrets
   and variables → Actions, add the secret `TAURI_SIGNING_PRIVATE_KEY` with the
   contents of that file:
   `gh secret set TAURI_SIGNING_PRIVATE_KEY < ~/.tauri/orbital-updater.key`
3. **Where downloads come from.** Installed copies download without signing
   in, so releases must be public:
   - a public repository: nothing more to do; or
   - private code, public downloads: create a public repository such as
     `you/orbital-releases` with one commit (a README), set the Actions
     variable `RELEASES_REPO` to `you/orbital-releases`, and add a secret
     `RELEASES_TOKEN`, a fine-grained token with Contents: read and write on
     that repository.
4. Optional: Windows code signing, so SmartScreen doesn't warn (Azure
   Trusted Signing or a code-signing certificate, set up under
   `bundle.windows` in `src-tauri/tauri.windows.conf.json`).
5. Optional: the `APPLE_*` secrets listed in `.github/workflows/release.yml`
   for a signed and notarized app. Without them macOS warns on first open
   (System Settings → Privacy & Security → Open Anyway, once).

The address installed copies check is written into each build by the release
workflow, from the repository it publishes to. A copy built before a move
keeps checking the old address, so choose the repository before giving the
app to anyone.

## Each release

1. Write what changed under `## Unreleased` in `CHANGELOG.md`; this becomes
   the release text and the "What's new" in the app. Commit it.
2. `npm run release -- minor` (or patch, major, beta, 1.4.0; add `--dry-run`
   to preview). It bumps the version everywhere, dates the changelog section,
   commits and tags.
3. `git push && git push origin v1.1.0`. The Release workflow builds a draft.
4. Check the draft: it has `Orbital_1.1.0_universal.dmg`, `Orbital.app.tar.gz`
   and its `.sig` (macOS), `Orbital_1.1.0_x64-setup.exe` and its `.sig`
   (Windows; the Windows job runs after the macOS one and adds to the same
   draft), and `latest.json` with both platforms. Publish it. Installed copies see it at their
   next check (launch, or within six hours); Settings → Check for updates
   finds it at once.

The first update can only be tried end to end with two published releases:
install the first from its `.dmg`, then publish the second.
