# shellcheck shell=bash
# Shared setup for the packaging scripts. Source it: `. "$(dirname "$0")/../env.sh"`.
#
# Exports:
#   ROOT                  workspace root
#   VERSION               [workspace.package] version from Cargo.toml (override: ABACO_VERSION)
#   DIST                  output directory for release artifacts (default: $ROOT/dist/release)
#   APP_ID                dev.danielmiguelt.Abaco: the desktop entry, metainfo, icons and bundle id
#   BUILD_DATE            UTC build date, YYYY-MM-DD (override: ABACO_BUILD_DATE)
#   CARGO_TARGET_DIR      cargo's target dir (default: $ROOT/target)
#   APP_CARGO_BIN         the desktop app's binary as cargo builds it (apps/abaco-app)
#   CLI_CARGO_BIN         the command line's binary as cargo builds it (apps/abaco-cli)
#   APP_BIN, CLI_BIN      the names the packages install them under
#
# Every Nodalia app installs its window as `<app>` and its command line as `<app>-cli`, and the
# Nodalia Launcher relies on that (it starts `abaco`, and looks for `abaco.exe` in a portable zip).
# Ábaco's crates are named the other way round (`abaco-app` is the window, `abaco` the command
# line), so the packages rename them while staging. If the crates are renamed, make the cargo and
# installed names equal here and nowhere else.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export ROOT

# The version lives in exactly one place: `[workspace.package] version` in the root Cargo.toml.
workspace_version() {
  awk '
    /^\[/ { in_pkg = ($0 == "[workspace.package]") ; next }
    in_pkg && $1 == "version" { gsub(/[" ]/, "", $3); print $3; exit }
  ' "$ROOT/Cargo.toml"
  return
}

VERSION="${ABACO_VERSION:-$(workspace_version)}"
if [[ -z "$VERSION" ]]; then
  echo "error: could not read [workspace.package] version from $ROOT/Cargo.toml" >&2
  exit 1
fi
export VERSION

DIST="${DIST:-$ROOT/dist/release}"
mkdir -p "$DIST"
export DIST

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
export APP_ID=dev.danielmiguelt.Abaco
export BUILD_DATE="${ABACO_BUILD_DATE:-$(date -u +%Y-%m-%d)}"
export APP_CARGO_BIN=abaco-app CLI_CARGO_BIN=abaco
export APP_BIN=abaco CLI_BIN=abaco-cli
# The cargo packages that hold those two binaries.
export APP_PACKAGE=abaco-app CLI_PACKAGE=abaco-cli

# Emit a GitHub Actions warning (plain stderr outside Actions).
warn() {
  if [[ -n "${GITHUB_ACTIONS:-}" ]]; then echo "::warning::$*"; else echo "warning: $*" >&2; fi
  return
}

# Copy licence and readme files that exist into a package directory.
copy_docs() {
  local dest="$1" f
  for f in README.md LICENSE LICENSE-MIT LICENSE-APACHE COPYRIGHT NOTICE ATTRIBUTION.md; do
    if [[ -f "$ROOT/$f" ]]; then cp "$ROOT/$f" "$dest/"; fi
  done
  return
}

# Portable SHA-256 of a file (prints just the hash).
sha256() {
  local file="$1"
  if command -v sha256sum >/dev/null; then sha256sum "$file" | cut -d' ' -f1; else shasum -a 256 "$file" | cut -d' ' -f1; fi
  return
}
