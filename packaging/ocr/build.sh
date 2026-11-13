#!/usr/bin/env bash
# Builds the OCR helper (Apple Vision) for both architectures, named the way
# Tauri's `externalBin` expects: binaries/stitchup-ocr-<target-triple>.
set -euo pipefail
cd "$(dirname "$0")"
OUT=../../source/app/src-tauri/binaries
TMP=$(mktemp -d)
mkdir -p "$OUT"
for a in arm64 x86_64; do
  # 10.13 deployment target (the app's minimum); recognition itself checks for 10.15 at run time.
  xcrun swiftc -O -target "$a-apple-macos10.13" main.swift -o "$TMP/ocr-$a"
done
cp "$TMP/ocr-arm64" "$OUT/stitchup-ocr-aarch64-apple-darwin"
cp "$TMP/ocr-x86_64" "$OUT/stitchup-ocr-x86_64-apple-darwin"
lipo -create "$TMP/ocr-arm64" "$TMP/ocr-x86_64" -output "$OUT/stitchup-ocr-universal-apple-darwin"
rm -rf "$TMP"
echo "OCR helper: $(lipo -archs "$OUT/stitchup-ocr-universal-apple-darwin")"
