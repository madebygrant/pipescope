#!/usr/bin/env bash
# Builds Pipescope from source into /Applications:
#   curl -fsSL https://raw.githubusercontent.com/madebygrant/pipescope/main/scripts/install.sh | bash
# Builds the latest release; PIPESCOPE_REF=main (or any tag or branch) picks another.
# Rust's build output is kept in ~/Library/Caches/pipescope, so updates rebuild in minutes.
set -euo pipefail

REPO="madebygrant/pipescope"
APP="/Applications/Pipescope.app"
CACHE="$HOME/Library/Caches/pipescope"

say() { printf '\033[1m%s\033[0m\n' "$*"; }
fail() {
  printf '\033[31m%s\033[0m\n' "$*" >&2
  exit 1
}

# Piped into bash, stdin is this script, so questions go to the terminal instead.
# With no terminal (CI, cron) every answer is no.
ask() {
  local reply
  { : </dev/tty; } 2>/dev/null || return 1
  printf '%s [y/N] ' "$1" >/dev/tty
  read -r reply </dev/tty || return 1
  [[ "$reply" =~ ^[Yy] ]]
}

[ "$(uname -s)" = "Darwin" ] || fail "Pipescope is a macOS app."

# rustup puts cargo here, and a fresh install hasn't reached this shell's PATH yet.
export PATH="$HOME/.cargo/bin:$PATH"

node_ok() {
  command -v node >/dev/null || return 1
  local major minor
  IFS=. read -r major minor _ <<<"$(node -p 'process.versions.node')"
  # Vite needs ^20.19 or >=22.12.
  ((major > 22 || (major == 22 && minor >= 12) || (major == 20 && minor >= 19)))
}

missing=()
xcode-select -p >/dev/null 2>&1 || missing+=(xcode)
command -v cargo >/dev/null || missing+=(rust)
node_ok || missing+=(node)
command -v pnpm >/dev/null || missing+=(pnpm)

how_to_install() {
  case "$1" in
    xcode) echo "Xcode command line tools: xcode-select --install" ;;
    rust) echo "Rust: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh" ;;
    node) echo "Node 20.19 or later: brew install node, or the installer from https://nodejs.org" ;;
    pnpm) echo "pnpm: npm install -g pnpm" ;;
  esac
}

install_one() {
  case "$1" in
    xcode)
      xcode-select --install || true
      # The tools install through a macOS dialog this script can't wait on.
      say "Finish the Xcode command line tools install in the dialog, then run this script again."
      exit 0
      ;;
    rust) curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal ;;
    node)
      command -v brew >/dev/null || fail "Installing Node here needs Homebrew. Get Node from https://nodejs.org, then run this again."
      brew install node
      ;;
    pnpm)
      node_ok || fail "pnpm needs Node first."
      npm install -g pnpm
      ;;
  esac
}

if ((${#missing[@]})); then
  say "Building Pipescope needs these, which aren't installed:"
  for tool in "${missing[@]}"; do echo "  - $(how_to_install "$tool")"; done
  echo "Rust and Pipescope's build cache take about 2 GB, and the first build takes 10 to 15 minutes."
  for tool in "${missing[@]}"; do
    if ask "Install $tool now?"; then
      install_one "$tool"
    else
      fail "Install the tools above, then run this again."
    fi
  done
fi

github() { curl -fsSL "https://api.github.com/$1"; }

ref="${PIPESCOPE_REF:-}"
if [ -z "$ref" ]; then
  ref=$(github "repos/$REPO/releases/latest" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n1)
  [ -n "$ref" ] || fail "Couldn't find the latest release. Set PIPESCOPE_REF=main to build main instead."
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

say "Downloading Pipescope $ref"
github "repos/$REPO/tarball/$ref" | tar -xz -C "$work" --strip-components=1
cd "$work"

say "Building (the first build takes 10 to 15 minutes)"
mkdir -p "$CACHE"
export CARGO_TARGET_DIR="$CACHE/target"
pnpm install --frozen-lockfile
pnpm tauri build --bundles app

built="$CARGO_TARGET_DIR/release/bundle/macos/Pipescope.app"
[ -d "$built" ] || fail "The build finished but $built is missing."

if pgrep -f "$APP/Contents/MacOS" >/dev/null; then
  say "Quitting Pipescope"
  osascript -e 'quit app "Pipescope"' >/dev/null 2>&1 || true
  for _ in 1 2 3 4 5; do
    pgrep -f "$APP/Contents/MacOS" >/dev/null || break
    sleep 1
  done
  pkill -f "$APP/Contents/MacOS" || true
fi

say "Installing to $APP"
# Copied alongside first, so a failed copy leaves the old app in place.
staged="/Applications/.Pipescope.app.new"
rm -rf "$staged"
cp -R "$built" "$staged"
rm -rf "$APP"
mv "$staged" "$APP"
# Without this, Finder and the Dock can keep showing an older build's icon.
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$APP" || true
open "$APP"

version=$(/usr/libexec/PlistBuddy -c "Print CFBundleShortVersionString" "$APP/Contents/Info.plist")
say "Pipescope $version is installed and running. Press ⌥⌘P to open it."
