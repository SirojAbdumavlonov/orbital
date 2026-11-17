#!/usr/bin/env bash
# Install the built .deb, start Orbital without a screen and check it runs:
# the binary's libraries resolve, the desktop entry is valid, the window
# process stays up, and it creates its data folder with a private vault.
#
#   bash scripts/linux-smoke.sh [path/to/Orbital.deb] [path/to/Orbital.AppImage]
#
# Needs: sudo, xvfb-run (xvfb), dbus-run-session (dbus), desktop-file-validate
# (desktop-file-utils). CI runs it after `npm run tauri build`.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
deb=${1:-$(ls "$root"/target/release/bundle/deb/*.deb | head -n1)}
appimage=${2:-$(ls "$root"/target/release/bundle/appimage/*.AppImage 2>/dev/null | head -n1 || true)}
fail() { echo "linux-smoke: FAIL: $*" >&2; exit 1; }
ok() { echo "linux-smoke: ok: $*"; }

echo "linux-smoke: installing $deb"
sudo apt-get install -y "$deb" >/dev/null
bin=$(command -v orbital) || fail "the package installed no 'orbital' command"
ok "installed $bin ($(dpkg-query -W -f='${Version}' orbital))"

missing=$(ldd "$bin" | grep 'not found' || true)
[ -z "$missing" ] || fail "libraries missing: $missing"
ok "all libraries resolve"

desktop=$(dpkg -L orbital | grep '\.desktop$' | head -n1)
[ -n "$desktop" ] || fail "no .desktop entry in the package"
desktop-file-validate "$desktop" || fail "invalid desktop entry $desktop"
grep -q '^Exec=orbital' "$desktop" || fail "desktop entry doesn't start orbital"
dpkg -L orbital | grep -q 'icons/hicolor/.*/apps/.*\.png' || fail "no icon in the package"
ok "desktop entry and icon"

# A fresh home, so the check doesn't depend on (or touch) the runner's.
home=$(mktemp -d)
log="$home/orbital.log"
echo "linux-smoke: starting Orbital under Xvfb (HOME=$home)"
HOME="$home" XDG_DATA_HOME="$home/.local/share" RUST_LOG=info \
  xvfb-run -a dbus-run-session -- "$bin" >"$log" 2>&1 &
pid=$!

data="$home/.local/share/orbital"
for _ in $(seq 1 40); do
  [ -f "$data/orbital.db" ] && [ -f "$data/.install-key" ] && break
  kill -0 "$pid" 2>/dev/null || { cat "$log"; fail "Orbital exited during start-up"; }
  sleep 0.5
done
[ -f "$data/orbital.db" ] || { cat "$log"; fail "no database in $data"; }
# The vault's key is made at start-up; vault.bin itself only once a secret is saved.
[ -f "$data/.install-key" ] || { cat "$log"; fail "no vault key in $data"; }
ok "created $data"

[ ! -f "$data/vault.bin" ] || [ "$(stat -c %a "$data/vault.bin")" = 600 ] || fail "vault.bin is not 600"
[ "$(stat -c %a "$data/.install-key")" = 600 ] || fail ".install-key is not 600"
[ "$(stat -c %a "$data")" = 700 ] || fail "$data is $(stat -c %a "$data"), not 700"
ok "data folder and vault key are private"

# Still running a few seconds later: the web view loaded instead of crashing.
sleep 5
kill -0 "$pid" 2>/dev/null || { cat "$log"; fail "Orbital exited after start-up"; }
if grep -Ei 'panicked|segmentation fault|failed to create webview' "$log"; then fail "errors in the log"; fi
ok "still running after start-up"
pkill -TERM -P "$pid" 2>/dev/null || true
kill "$pid" 2>/dev/null || true
wait "$pid" 2>/dev/null || true

if [ -n "$appimage" ]; then
  chmod +x "$appimage"
  # FUSE isn't available on CI runners; the AppImage can unpack itself instead.
  APPIMAGE_EXTRACT_AND_RUN=1 "$appimage" --appimage-help >/dev/null 2>&1 \
    || file "$appimage" | grep -q ELF || fail "$appimage is not a runnable AppImage"
  ok "AppImage $(basename "$appimage")"
fi

sudo apt-get remove -y orbital >/dev/null
command -v orbital >/dev/null && fail "orbital is still installed after removal"
ok "removed cleanly"
echo "linux-smoke: all checks passed"
