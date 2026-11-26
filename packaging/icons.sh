#!/usr/bin/env bash
# Regenerate every installer icon (Linux hicolor PNGs, Windows .ico, macOS .icns, the 1024 px
# master) into assets/app-icon from the code that draws the window icon.
#
# Needs only cargo. The outputs are committed, so building and packaging never need this; run it
# after changing apps/abaco-app/src/icon.rs, or after replacing the picture with another one.
#
#   packaging/icons.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
cargo run -q --locked -p abaco-app --example icons -- "$ROOT/assets/app-icon"
