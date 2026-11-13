#!/usr/bin/env bash
# Stitchup — build, test and package the macOS app.
#   ./build-macos.sh          tests + universal (Apple Silicon + Intel) app -> dist/
#   ./build-macos.sh --quick  skip the tests
# Uses the workspace's portable toolchain (../tools/env.sh): Rust, Node, Playwright.
set -euo pipefail
cd "$(dirname "$0")"
source ../tools/env.sh

APP_DIR=source/app
VERSION=$(sed -n 's/.*"version": "\(.*\)".*/\1/p' "$APP_DIR/src-tauri/tauri.conf.json" | head -1)

echo "==> OCR helper (Swift, Apple Vision)"
./packaging/ocr/build.sh

if [ "${1:-}" != "--quick" ]; then
  echo "==> Core + command-layer tests"
  (cd source/core && cargo test --release --quiet)
  (cd "$APP_DIR/src-tauri" && cargo test --release --quiet)
  echo "==> Frontend build + end-to-end UI test (real backend via dev bridge)"
  (cd "$APP_DIR" && npm run build >/dev/null && npm run test:e2e)
fi

echo "==> Universal app (v$VERSION)"
(cd "$APP_DIR" && npx tauri build --target universal-apple-darwin)
BUNDLE="$APP_DIR/src-tauri/target/universal-apple-darwin/release/bundle/macos/Stitchup.app"
codesign --force --deep -s - "$BUNDLE"
codesign --verify --deep --strict "$BUNDLE"
if otool -L "$BUNDLE/Contents/MacOS/stitchup" | grep -E '^\s' | grep -v -E '/usr/lib/|/System/Library/' | grep -q .; then
  echo "ERROR: links a non-system library"; exit 1
fi

echo "==> Publishing dist/ (replaces the previous build)"
rm -rf dist && mkdir dist
ditto "$BUNDLE" dist/Stitchup.app
(cd dist && ditto -c -k --keepParent Stitchup.app "Stitchup-macOS-universal-v$VERSION.zip")
echo "==> Done: dist/Stitchup.app ($(lipo -archs dist/Stitchup.app/Contents/MacOS/stitchup))"
