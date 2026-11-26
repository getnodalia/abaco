#!/usr/bin/env bash
# Build and sign the macOS release artifacts:
#
#   $DIST/abaco-<version>-macos-<arch>.dmg          Abaco.app on a drag-to-Applications DMG
#   $DIST/abaco-cli-<version>-macos-<arch>.zip      the headless command line
#
# Usage: packaging/macos/package.sh [--arch universal|aarch64|x86_64] [--skip-build]
#
# Signing (env): MACOS_SIGN_IDENTITY is a codesign identity (name or SHA-1) and MACOS_KEYCHAIN the
# keychain that holds it (optional). Unset, the artifacts are signed ad hoc ("-"): there is no
# Developer ID and no notarization, so Gatekeeper asks for right click > Open the first time.
# (Ad-hoc code runs on Apple silicon, which refuses completely unsigned code.)
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/macos"

ARCH=universal
SKIP_BUILD=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --arch) ARCH="$2"; shift 2 ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    -h | --help) sed -n '2,14p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
case "$ARCH" in
  universal) TARGETS=(aarch64-apple-darwin x86_64-apple-darwin) ;;
  aarch64) TARGETS=(aarch64-apple-darwin) ;;
  x86_64) TARGETS=(x86_64-apple-darwin) ;;
  *) echo "unknown --arch $ARCH" >&2; exit 2 ;;
esac

# Keep in sync with LSMinimumSystemVersion in Info.plist.in.
export MACOSX_DEPLOYMENT_TARGET=11.0
IDENTITY="${MACOS_SIGN_IDENTITY:--}"
SHORT_VERSION="${VERSION%%-*}"
WORK="$CARGO_TARGET_DIR/macos-package"
APP="$WORK/Abaco.app"
DMG="$DIST/abaco-$VERSION-macos-$ARCH.dmg"
CLI_ZIP="$DIST/abaco-cli-$VERSION-macos-$ARCH.zip"
BUILD_SHA="${ABACO_BUILD_SHA:-$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || echo unknown)}"

if [[ "$IDENTITY" = "-" ]]; then
  warn "macOS: ad-hoc signing (no MACOS_SIGN_IDENTITY); the artifacts are not notarized"
fi

echo "==> Ábaco $VERSION for macOS ($ARCH), identity: $IDENTITY"

# ---- build -------------------------------------------------------------------------------------
if [[ "$SKIP_BUILD" = 0 ]]; then
  args=()
  for t in "${TARGETS[@]}"; do args+=(--target "$t"); done
  (cd "$ROOT" && cargo build --release --locked -p "$APP_PACKAGE" -p "$CLI_PACKAGE" "${args[@]}")
fi

rm -rf "$WORK"
mkdir -p "$WORK/bin"
# A universal binary is the two architectures joined; a single one is copied as it is.
for pair in "$APP_CARGO_BIN:$APP_BIN" "$CLI_CARGO_BIN:$CLI_BIN"; do
  cargo_bin="${pair%%:*}"
  installed="${pair##*:}"
  inputs=()
  for t in "${TARGETS[@]}"; do inputs+=("$CARGO_TARGET_DIR/$t/release/$cargo_bin"); done
  lipo -create -output "$WORK/bin/$installed" "${inputs[@]}"
  lipo -info "$WORK/bin/$installed"
done

# ---- signing helper ----------------------------------------------------------------------------
sign() {
  # Hardened runtime + secure timestamp for a real identity; ad-hoc can't be timestamped.
  local ts=(--timestamp)
  [[ "$IDENTITY" = "-" ]] && ts=(--timestamp=none)
  local kc=()
  [[ -n "${MACOS_KEYCHAIN:-}" ]] && kc=(--keychain "$MACOS_KEYCHAIN")
  codesign --force --sign "$IDENTITY" ${kc[@]+"${kc[@]}"} "${ts[@]}" "$@"
}

# ---- Abaco.app ---------------------------------------------------------------------------------
echo "==> assembling $APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
# The executable and the icon carry the bundle's name (CFBundleExecutable / CFBundleIconFile). The
# command line sits beside it as abaco-cli, since `Abaco` and `abaco` are one name on a Mac's disk.
cp "$WORK/bin/$APP_BIN" "$APP/Contents/MacOS/Abaco"
cp "$WORK/bin/$CLI_BIN" "$APP/Contents/MacOS/$CLI_BIN"
cp "$ROOT/assets/app-icon/abaco.icns" "$APP/Contents/Resources/Abaco.icns"
copy_docs "$APP/Contents/Resources"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@SHORT_VERSION@/$SHORT_VERSION/g" \
  -e "s/@BUILD_SHA@/$BUILD_SHA/g" \
  "$HERE/Info.plist.in" >"$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist"
# An executable or icon name that doesn't match a file leaves the app unable to open, with a
# crossed-out icon in the DMG. Check them here rather than on someone's Mac.
python3 - "$APP" <<'PY'
import os, plistlib, sys
app = sys.argv[1]
with open(os.path.join(app, "Contents", "Info.plist"), "rb") as f:
    info = plistlib.load(f)
missing = [p for p in (os.path.join("MacOS", info["CFBundleExecutable"]),
                       os.path.join("Resources", info["CFBundleIconFile"] + ".icns"))
           if not os.path.isfile(os.path.join(app, "Contents", p))]
if missing:
    sys.exit(f"error: Info.plist names files the bundle lacks: {missing}")
PY
printf 'APPL????' >"$APP/Contents/PkgInfo"

# Sign inside-out: nested code first, then the bundle itself (no --deep on the final signature).
sign --options runtime "$APP/Contents/MacOS/$CLI_BIN"
sign --options runtime --entitlements "$HERE/entitlements.plist" "$APP/Contents/MacOS/Abaco"
sign --options runtime --entitlements "$HERE/entitlements.plist" "$APP"
codesign --verify --strict --deep --verbose=2 "$APP"
"$APP/Contents/MacOS/$CLI_BIN" --version

# ---- DMG ---------------------------------------------------------------------------------------
echo "==> building $DMG"
STAGE="$WORK/dmg"
mkdir -p "$STAGE"
ditto "$APP" "$STAGE/Abaco.app"
ln -s /Applications "$STAGE/Applications"
# Finder window layout: background, icon size and positions (packaging/macos/dmg/README.md).
mkdir -p "$STAGE/.background"
cp "$HERE/dmg/background.tiff" "$STAGE/.background/background.tiff"
# The layout names the volume (the background's alias) and the app (its icon position). A layout
# left over from another name opens a plain window with no background, so stop the build instead.
python3 - "$HERE/dmg/dmg-layout.DS_Store" "Abaco" "Abaco.app" <<'PY'
import sys
path, volume, app = sys.argv[1:4]
data = open(path, "rb").read()
if f"/Volumes/{volume}".encode() not in data or app.encode("utf-16-be") not in data:
    sys.exit(f"error: {path} is laid out for another volume or app; run packaging/macos/dmg/generate.py")
PY
cp "$HERE/dmg/dmg-layout.DS_Store" "$STAGE/.DS_Store"
rm -f "$DMG"
# `hdiutil create -srcfolder` writes the files as they are. `makehybrid -hfs` gave every file a
# com.apple.FinderInfo, which `codesign --verify --strict` rejects as "Finder information, or
# similar detritus" on the app in the image and on a copy installed from it. `create` attaches a
# device while it copies, which fails now and then on CI runners ("Resource busy"), so it gets a
# few tries. The volume name has no version: .DS_Store finds the background through an alias that
# includes it.
# Usage: hdiutil_retry <what> <hdiutil args…>; tries five times with growing pauses.
hdiutil_retry() {
  local what="$1" attempt
  shift
  for attempt in 1 2 3 4 5; do
    if hdiutil "$@"; then
      return 0
    fi
    [[ "$attempt" = 5 ]] && { echo "hdiutil $what failed 5 times" >&2; return 1; }
    echo "hdiutil $what failed (attempt $attempt of 5); retrying in $((attempt * 10))s" >&2
    sleep $((attempt * 10))
  done
}
hdiutil_retry create create -srcfolder "$STAGE" -volname "Abaco" -fs HFS+ -format UDZO -imagekey zlib-level=9 -ov "$DMG"
sign "$DMG"
codesign --verify --strict --verbose=2 "$DMG"
# The app inside the image must still pass --strict, so a makehybrid-style regression can't ship.
MOUNT="$WORK/dmg-check"
rm -rf "$MOUNT"
mkdir -p "$MOUNT"
hdiutil_retry attach attach -nobrowse -readonly -mountpoint "$MOUNT" "$DMG" >/dev/null
if ! codesign --verify --strict --deep --verbose=2 "$MOUNT/Abaco.app"; then
  hdiutil_retry detach detach "$MOUNT" >/dev/null || true
  exit 1
fi
hdiutil_retry detach detach "$MOUNT" >/dev/null

# ---- command line ------------------------------------------------------------------------------
echo "==> building $CLI_ZIP"
CLI_DIR="$WORK/abaco-cli-$VERSION-macos-$ARCH"
mkdir -p "$CLI_DIR"
cp "$WORK/bin/$CLI_BIN" "$CLI_DIR/"
copy_docs "$CLI_DIR"
sign --options runtime "$CLI_DIR/$CLI_BIN"
codesign --verify --strict --verbose=2 "$CLI_DIR/$CLI_BIN"
rm -f "$CLI_ZIP"
ditto -c -k --keepParent "$CLI_DIR" "$CLI_ZIP"

"$CLI_DIR/$CLI_BIN" --version
echo "==> done"
ls -lh "$DMG" "$CLI_ZIP"
