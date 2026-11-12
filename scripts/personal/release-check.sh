#!/usr/bin/env bash
# Local macOS release gate, invoked by maintain.py verify on the candidate.
set -euo pipefail
cd "$(dirname "$0")/../.."

if [[ "$(uname -s)" != Darwin ]]; then
    echo 'Hex Ralf releases must be checked on the local macOS host.' >&2
    exit 1
fi

# Keep tests bounded and use this checkout's binary for the CA scan, regardless
# of a caller's shared Cargo target directory.
command -v cargo-nextest >/dev/null
export NEXTEST_TEST_THREADS=4
export CARGO_TARGET_DIR="$PWD/target"
xcrun --sdk macosx metal --version
bash scripts/check
bash scripts/personal/check.sh
bash scripts/test-app-identity.sh
cargo build --locked --bin hex
bash scripts/check-ca-roots.sh "$CARGO_TARGET_DIR/debug/hex"
