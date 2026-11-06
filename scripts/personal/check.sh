#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
python3 -m unittest discover -s scripts/personal -p 'test_*.py'
# Add focused personal-feature regression commands here as features are introduced.
