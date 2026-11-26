#!/usr/bin/env bash
# Build and package Ábaco for Linux x86_64 or aarch64:
#
#   $DIST/abaco-<version>-linux-<arch>.AppImage        the desktop app, any distro with glibc >= the build host's
#   $DIST/abaco-<version>-linux-<arch>.AppImage.zsync  delta updates (needs zsyncmake)
#   $DIST/abaco-<version>-linux-<arch>.deb             Debian, Ubuntu, Mint, Pop!_OS, ...
#   $DIST/abaco-<version>-linux-<arch>.rpm             Fedora, openSUSE, RHEL, ...
#   $DIST/abaco-<version>-linux-<arch>.tar.gz          a plain bin/ + share/ tree
#
# Every package installs the desktop app as `abaco` and the command line as `abaco-cli` (see
# packaging/env.sh), with the desktop entry, AppStream metainfo, MIME type and hicolor icons.
#
# Usage: packaging/linux/package.sh [--skip-build] [--formats "appimage deb rpm tar"]
#
# Needs: cargo; nfpm for deb/rpm (https://nfpm.goreleaser.com). The AppImage tooling is downloaded
# at verified pins into $CARGO_TARGET_DIR. Build on an old distro (CI: Ubuntu 22.04, glibc 2.35)
# so the binaries run on newer ones. Optional: desktop-file-validate, appstreamcli, zsyncmake.
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/linux"
# shellcheck source=verified-download.sh
. "$HERE/verified-download.sh"

SKIP_BUILD=0
FORMATS="appimage deb rpm tar"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    --formats) FORMATS="$2"; shift 2 ;;
    -h | --help) sed -n '2,16p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

ARCH="$(uname -m)"
case "$ARCH" in
  x86_64) DEB_ARCH=amd64 ;;
  aarch64 | arm64) ARCH=aarch64; DEB_ARCH=arm64 ;;
  *) echo "unsupported architecture $ARCH" >&2; exit 2 ;;
esac
export ABACO_MAINTAINER="${ABACO_MAINTAINER:-Nodalia <https://github.com/getnodalia/abaco/issues>}"
BASENAME="abaco-$VERSION-linux-$ARCH"

echo "==> Ábaco $VERSION for Linux $ARCH ($FORMATS)"

if [[ "$SKIP_BUILD" = 0 ]]; then
  (cd "$ROOT" && cargo build --release --locked -p "$APP_PACKAGE" -p "$CLI_PACKAGE")
fi
BIN="$CARGO_TARGET_DIR/release"
WORK="$CARGO_TARGET_DIR/linux-package"
STAGE="$WORK/root"
rm -rf "$WORK"

# ---- stage an FHS tree (shared by every format) -------------------------------------------------
install -Dm755 "$BIN/$APP_CARGO_BIN" "$STAGE/usr/bin/$APP_BIN"
install -Dm755 "$BIN/$CLI_CARGO_BIN" "$STAGE/usr/bin/$CLI_BIN"
strip "$STAGE/usr/bin/$APP_BIN" "$STAGE/usr/bin/$CLI_BIN" 2>/dev/null || true
install -Dm644 "$HERE/$APP_ID.desktop" "$STAGE/usr/share/applications/$APP_ID.desktop"
install -Dm644 "$HERE/$APP_ID.mime.xml" "$STAGE/usr/share/mime/packages/$APP_ID.xml"
mkdir -p "$STAGE/usr/share/metainfo"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@DATE@/$BUILD_DATE/g" \
  "$HERE/$APP_ID.metainfo.xml.in" >"$STAGE/usr/share/metainfo/$APP_ID.metainfo.xml"
mkdir -p "$STAGE/usr/share/icons"
cp -R "$ROOT/assets/app-icon/hicolor" "$STAGE/usr/share/icons/"
mkdir -p "$STAGE/usr/share/doc/abaco"
copy_docs "$STAGE/usr/share/doc/abaco"

if command -v desktop-file-validate >/dev/null; then
  desktop-file-validate "$STAGE/usr/share/applications/$APP_ID.desktop"
fi
if command -v appstreamcli >/dev/null; then
  appstreamcli validate --no-net --explain "$STAGE/usr/share/metainfo/$APP_ID.metainfo.xml"
fi

has() { case " $FORMATS " in *" $1 "*) return 0 ;; *) return 1 ;; esac; }

# ---- .tar.gz ------------------------------------------------------------------------------------
if has tar; then
  mkdir -p "$WORK/tar"
  cp -R "$STAGE/usr" "$WORK/tar/$BASENAME"
  tar -C "$WORK/tar" -czf "$DIST/$BASENAME.tar.gz" "$BASENAME"
  echo "wrote $DIST/$BASENAME.tar.gz"
fi

# ---- .deb / .rpm --------------------------------------------------------------------------------
if has deb || has rpm; then
  command -v nfpm >/dev/null || { echo "error: nfpm not found (https://nfpm.goreleaser.com/install/)" >&2; exit 1; }
  export VERSION
  export NFPM_ARCH="$DEB_ARCH"
  # nfpm expands env vars in fields like `version` and `arch`, but not in `contents[].src`.
  sed -e "s|\${STAGE}|$STAGE|g" -e "s|@GLIBC_FLOOR@|${GLIBC_FLOOR:-2.35}|g" \
    "$HERE/nfpm.yaml" >"$WORK/nfpm.yaml"
  for fmt in deb rpm; do
    if has "$fmt"; then (cd "$ROOT" && nfpm package -f "$WORK/nfpm.yaml" -p "$fmt" -t "$DIST/$BASENAME.$fmt"); fi
  done
fi

# ---- AppImage -----------------------------------------------------------------------------------
if has appimage; then
  APPDIR="$WORK/Abaco.AppDir"
  mkdir -p "$APPDIR/usr/bin"
  cp "$STAGE/usr/bin/$APP_BIN" "$APPDIR/usr/bin/$APP_BIN"
  cp "$STAGE/usr/bin/$CLI_BIN" "$APPDIR/usr/bin/$CLI_BIN"
  cp -R "$STAGE/usr/share" "$APPDIR/usr/share"
  ln -s "usr/bin/$APP_BIN" "$APPDIR/AppRun"
  cp "$HERE/$APP_ID.desktop" "$APPDIR/$APP_ID.desktop"
  cp "$ROOT/assets/app-icon/hicolor/256x256/apps/$APP_ID.png" "$APPDIR/$APP_ID.png"
  ln -s "$APP_ID.png" "$APPDIR/.DirIcon"

  # Digests from the official GitHub release asset metadata:
  # https://github.com/AppImage/appimagetool/releases/tag/1.9.1
  # https://github.com/AppImage/type2-runtime/releases/tag/20251108
  # Keep both pins together: appimagetool otherwise downloads the mutable latest type2 runtime.
  TOOL_VERSION=1.9.1
  RUNTIME_VERSION=20251108
  case "$ARCH" in
    x86_64)
      TOOL_SHA256=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
      RUNTIME_SHA256=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d
      ;;
    aarch64)
      TOOL_SHA256=f0837e7448a0c1e4e650a93bb3e85802546e60654ef287576f46c71c126a9158
      RUNTIME_SHA256=00cbdfcf917cc6c0ff6d3347d59e0ca1f7f45a6df1a428a0d6d8a78664d87444
      ;;
  esac
  mkdir -p "$CARGO_TARGET_DIR"
  TOOL="${APPIMAGETOOL:-$CARGO_TARGET_DIR/appimagetool-$TOOL_VERSION-$ARCH.AppImage}"
  if [[ -n "${APPIMAGETOOL:-}" ]]; then
    verify_appimagetool_override "$TOOL" "$TOOL_SHA256" "$TOOL_VERSION" "$ARCH"
  else
    download_verified \
      "https://github.com/AppImage/appimagetool/releases/download/$TOOL_VERSION/appimagetool-$ARCH.AppImage" \
      "$TOOL" "$TOOL_SHA256" true
  fi
  RUNTIME="$CARGO_TARGET_DIR/type2-runtime-$RUNTIME_VERSION-$ARCH"
  download_verified \
    "https://github.com/AppImage/type2-runtime/releases/download/$RUNTIME_VERSION/runtime-$ARCH" \
    "$RUNTIME" "$RUNTIME_SHA256"
  # Absolute, because appimagetool runs in $DIST below.
  OUT="$(cd "$DIST" && pwd)/$BASENAME.AppImage"
  APPDIR="$(cd "$APPDIR" && pwd)"
  TOOL="$(cd "$(dirname "$TOOL")" && pwd)/$(basename "$TOOL")"
  RUNTIME="$(cd "$(dirname "$RUNTIME")" && pwd)/$(basename "$RUNTIME")"
  # A .zsync left by an earlier run would hide a missing zsyncmake and describe another file.
  rm -f "$OUT.zsync"
  # Update information: AppImageUpdate fetches only the blocks that changed in a newer release,
  # through the .zsync published next to each AppImage on GitHub Releases. `latest` is the newest
  # published release that is not a pre-release.
  REPO="${GITHUB_REPOSITORY:-getnodalia/abaco}"
  UPDATE_INFO="gh-releases-zsync|${REPO%%/*}|${REPO#*/}|latest|abaco-*-linux-$ARCH.AppImage.zsync"
  # Extract-and-run works without FUSE (containers, CI); the output embeds the static runtime.
  (cd "$DIST" && ARCH="$ARCH" APPIMAGE_EXTRACT_AND_RUN=1 "$TOOL" --no-appstream \
    --runtime-file "$RUNTIME" -u "$UPDATE_INFO" "$APPDIR" "$OUT")
  echo "wrote $OUT"
  if [[ -s "$OUT.zsync" ]]; then
    echo "wrote $OUT.zsync"
  else
    warn "zsyncmake not found, so $OUT.zsync was not written; AppImage delta updates need it"
  fi
fi

"$STAGE/usr/bin/$APP_BIN" --version
"$STAGE/usr/bin/$CLI_BIN" --version
echo "==> done"
ls -lh "$DIST"
