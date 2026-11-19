#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
python3 -m unittest discover -s scripts/personal -p 'test_*.py'
# Local recording recovery extends the existing capture and ordered output.
NEXTEST_TEST_THREADS=4 scripts/check 'recovery::' 'dictation::audio::recovery' \
    'recorded_prefix_reconciles_to_delayed_release_for_recovery' \
    'parakeet::output::tests' 'history::tests' 'rune_ui::tests::recovery'
hex_package=$(cargo pkgid)
cargo nextest run --locked -p "hex@${hex_package##*[#@]}" -E 'test(rune_hud::recovery::) | test(dictation::indicator::)'
# Explicit GUI regression against an already-built debug executable. It uses
# throwaway data and synthetic HUD states; it never opens a real microphone.
if [[ -n "${HEX_HUD_NATIVE_BINARY:-}" ]]; then
    python3 scripts/personal/check_hud_recovery.py "$HEX_HUD_NATIVE_BINARY"
fi
