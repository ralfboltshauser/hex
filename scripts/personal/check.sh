#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
python3 -m unittest discover -s scripts/personal -p 'test_*.py'
# Local recording recovery extends the existing capture and ordered output.
NEXTEST_TEST_THREADS=4 scripts/check 'recovery::' 'dictation::audio::recovery' \
    'recorded_prefix_reconciles_to_delayed_release_for_recovery' \
    'parakeet::output::tests' 'history::tests' 'rune_ui::tests::recovery'
