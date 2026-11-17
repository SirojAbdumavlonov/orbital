#!/usr/bin/env bash
# Install (or update) Orbital on macOS or Linux from its latest GitHub release.
#
#   curl -fsSL https://raw.githubusercontent.com/SirojAbdumavlonov/orbital/main/install.sh | bash
#
# Why a script: a browser marks what it downloads as "from the internet", and
# macOS then refuses an app that isn't notarized by Apple. A download made with
# curl carries no such mark, so Orbital opens normally. The script only fetches
# files from this repository's releases and checks each one against the SHA-256
# checksum GitHub publishes for it (when GitHub's API answers) before using it.
#
# Settings (environment variables):
#   ORBITAL_VERSION=1.0.1        a specific release instead of the latest
#   ORBITAL_INSTALL_DIR=DIR      macOS: where Orbital.app goes (default /Applications,
#                                or ~/Applications if /Applications isn't writable)
#                                Linux AppImage: where `orbital` goes (default ~/.local/bin)
#   ORBITAL_PACKAGE=deb|rpm|appimage   Linux: force a package type
#   GITHUB_TOKEN=…               optional; lifts GitHub's API limit of 60 requests an hour
set -euo pipefail

REPO="SirojAbdumavlonov/orbital"
say() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
die() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "this needs '$1', which isn't installed"; }

need curl
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# The GitHub API gives each file's SHA-256, but allows only 60 requests an
# hour per address without a token (shared networks and CI runners hit that).
# Without it, the version comes from the releases page and files by their name.
auth=()
[ -n "${GITHUB_TOKEN:-}" ] && auth=(-H "Authorization: Bearer $GITHUB_TOKEN")
if [ -n "${ORBITAL_VERSION:-}" ]; then
  api="https://api.github.com/repos/$REPO/releases/tags/v${ORBITAL_VERSION#v}"
else
  api="https://api.github.com/repos/$REPO/releases/latest"
fi
version=""
if [ -z "${ORBITAL_NO_API:-}" ] && curl -fsSL -H "Accept: application/vnd.github+json" ${auth[@]+"${auth[@]}"} "$api" -o "$tmp/release.json" 2>/dev/null; then
  version=$(sed -n 's/.*"tag_name": *"v\{0,1\}\([^"]*\)".*/\1/p' "$tmp/release.json" | head -n1)
fi
if [ -z "$version" ]; then
  : > "$tmp/release.json"
  if [ -n "${ORBITAL_VERSION:-}" ]; then
    version=${ORBITAL_VERSION#v}
  else
    # https://github.com/OWNER/REPO/releases/latest redirects to …/releases/tag/vX.Y.Z
    latest=$(curl -fsSI -o /dev/null -w '%{redirect_url}' "https://github.com/$REPO/releases/latest") \
      || die "couldn't reach GitHub to find the latest release"
    version=${latest##*/tag/}
    version=${version#v}
    case "$version" in ""|http*) die "no published release found at https://github.com/$REPO/releases" ;; esac
  fi
  say "GitHub's API isn't available right now; downloading by file name (no checksum check)" >&2
fi

# "url sha256" for every file of the release (sha256 is empty if GitHub has none).
assets() {
  awk '
    /"name":/ { d = "" }
    /"digest":/ { if (match($0, /sha256:[0-9a-f]+/)) d = substr($0, RSTART + 7, RLENGTH - 7) }
    /"browser_download_url":/ { if (match($0, /https:[^"]+/)) print substr($0, RSTART, RLENGTH), d }
  ' "$tmp/release.json"
}

# Download the release file whose name matches the regex $1 (or, without the
# API, is called $2) into $tmp; prints its path.
fetch() {
  local line url sum file got
  if [ -s "$tmp/release.json" ]; then
    line=$(assets | grep -E "/$1 " | head -n1 || true)
    [ -n "$line" ] || die "Orbital $version has no file matching $1"
    url=${line%% *}
    sum=${line#* }
  else
    url="https://github.com/$REPO/releases/download/v$version/$2"
    sum=""
  fi
  file="$tmp/$(basename "$url")"
  say "Downloading $(basename "$url")" >&2
  curl -fL --progress-bar "$url" -o "$file" || die "download failed: $url"
  if [ -n "$sum" ]; then
    if command -v sha256sum >/dev/null 2>&1; then got=$(sha256sum "$file" | cut -d' ' -f1)
    else got=$(shasum -a 256 "$file" | cut -d' ' -f1); fi
    [ "$got" = "$sum" ] || die "checksum mismatch for $(basename "$file"): the download is damaged or was changed"
    say "Checksum verified" >&2
  fi
  printf '%s\n' "$file"
}

as_root() {
  if [ "$(id -u)" = 0 ]; then "$@"
  else need sudo; sudo "$@"; fi
}

install_macos() {
  need hdiutil
  local dmg mnt dest app
  dmg=$(fetch 'Orbital_[^/]*_universal\.dmg' "Orbital_${version}_universal.dmg")
  if [ -n "${ORBITAL_INSTALL_DIR:-}" ]; then dest=$ORBITAL_INSTALL_DIR
  elif [ -w /Applications ]; then dest=/Applications
  else dest="$HOME/Applications"; fi
  mkdir -p "$dest"
  app="$dest/Orbital.app"

  mnt=$(mktemp -d "$tmp/mnt.XXXX")
  hdiutil attach -nobrowse -readonly -quiet -mountpoint "$mnt" "$dmg" || die "couldn't open the disk image"
  [ -d "$mnt/Orbital.app" ] || { hdiutil detach -quiet "$mnt"; die "the disk image has no Orbital.app"; }

  if pgrep -xq Orbital 2>/dev/null; then
    say "Quitting the running Orbital"
    osascript -e 'quit app "Orbital"' >/dev/null 2>&1 || true
    sleep 2
  fi
  say "Installing to $app"
  rm -rf "$app"
  ditto "$mnt/Orbital.app" "$app"
  hdiutil detach -quiet "$mnt" || true
  # Nothing set the flag, but a copy made from a browser download earlier might have.
  xattr -dr com.apple.quarantine "$app" 2>/dev/null || true

  say "Orbital $version is installed. Open it from Launchpad or: open \"$app\""
}

install_linux() {
  local arch pkg file dest
  arch=$(uname -m)
  [ "$arch" = x86_64 ] || die "Orbital for Linux is built for x86_64 only (this machine is $arch)"
  pkg=${ORBITAL_PACKAGE:-}
  if [ -z "$pkg" ]; then
    if command -v apt-get >/dev/null 2>&1 && command -v dpkg >/dev/null 2>&1; then pkg=deb
    elif command -v dnf >/dev/null 2>&1 || command -v zypper >/dev/null 2>&1 || command -v yum >/dev/null 2>&1; then pkg=rpm
    else pkg=appimage; fi
  fi
  case "$pkg" in
    deb)
      file=$(fetch 'Orbital_[^/]*_amd64\.deb' "Orbital_${version}_amd64.deb")
      say "Installing the .deb (asks for your password)"
      as_root apt-get install -y "$file"
      ;;
    rpm)
      # Pre-release versions (with a "-") are published without an .rpm.
      if { [ -s "$tmp/release.json" ] && ! assets | grep -qE '/Orbital-[^/]*\.x86_64\.rpm '; } \
        || { [ ! -s "$tmp/release.json" ] && [ "${version#*-}" != "$version" ]; }; then
        say "Orbital $version has no .rpm; using the AppImage"
        ORBITAL_PACKAGE=appimage install_linux
        return
      fi
      file=$(fetch 'Orbital-[^/]*\.x86_64\.rpm' "Orbital-${version}-1.x86_64.rpm")
      say "Installing the .rpm (asks for your password)"
      if command -v dnf >/dev/null 2>&1; then as_root dnf install -y "$file"
      elif command -v zypper >/dev/null 2>&1; then as_root zypper --non-interactive install --allow-unsigned-rpm "$file"
      else as_root yum install -y "$file"; fi
      ;;
    appimage)
      file=$(fetch 'Orbital_[^/]*_amd64\.AppImage' "Orbital_${version}_amd64.AppImage")
      dest=${ORBITAL_INSTALL_DIR:-$HOME/.local/bin}
      mkdir -p "$dest"
      install -m 755 "$file" "$dest/orbital"
      say "Installed $dest/orbital (the AppImage updates itself)"
      case ":$PATH:" in *":$dest:"*) ;; *) say "Add $dest to your PATH to start it with: orbital" ;; esac
      ;;
    *) die "ORBITAL_PACKAGE must be deb, rpm or appimage" ;;
  esac
  say "Orbital $version is installed. Start it from your applications menu or run: orbital"
}

say "Orbital $version"
case "$(uname -s)" in
  Darwin) install_macos ;;
  Linux) install_linux ;;
  *) die "this script is for macOS and Linux; on Windows use install.ps1 (see the README)" ;;
esac
