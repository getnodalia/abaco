#!/usr/bin/env bash
# Regenerate the Windows .ico and macOS .icns in assets/app-icon from the PNGs in assets/icons.
#
# Needs only cargo. The outputs are committed, so building and packaging never need this; run it
# after replacing the PNGs in assets/icons.
#
#   packaging/icons.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
cargo run -q --locked -p abaco-app --example icons -- "$ROOT/assets/app-icon"
