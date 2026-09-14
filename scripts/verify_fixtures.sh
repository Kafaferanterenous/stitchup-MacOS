#!/usr/bin/env sh
# Verify every fixture against the acceptance-subset assertions:
# reloads all fixtures, page counts/sizes, rotations, encryption flags,
# annotations, the malformed F-08 (must not panic), F-11 (200 pages), F-13/14.
# Run from the repository ROOT.
#
#   ./scripts/verify_fixtures.sh
#
# Exit 0 only when verify_fixtures reports 0 failures.

set -e
cd "$(dirname "$0")/.."

cargo run --quiet --release --manifest-path source/core/Cargo.toml --bin verify_fixtures