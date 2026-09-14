#!/usr/bin/env sh
# Regenerate the sanitized fixture PDFs (F-01..F-14, minus deferred F-09) into
# fixtures/generated/ using the Rust core's fixture generator.
# Run from the repository ROOT, not from scripts/.
#
#   ./scripts/generate_fixtures.sh
#
# Output: fixtures/generated/F-*.pdf (written fresh each run; do not hand-edit).

set -e
cd "$(dirname "$0")/.."

cargo run --quiet --release --manifest-path source/core/Cargo.toml --bin generate_fixtures

echo "fixtures regenerated in fixtures/generated/"