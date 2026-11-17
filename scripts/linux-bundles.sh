#!/usr/bin/env bash
# The Linux bundles to build for the app's version. RPM forbids "-" in a
# version, so a pre-release such as 1.0.0-2 or 1.1.0-beta.1 ships as .deb and
# AppImage only; the .rpm comes back with the next plain version (1.0.1).
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
v=$(node -p "require('$root/src-tauri/tauri.conf.json').version")
case "$v" in
  *-*) echo "deb,appimage" ;;
  *) echo "deb,rpm,appimage" ;;
esac
