# Stitchup app icons

Placeholder until the build AIs generate real icons:

    cargo tauri icon path/to/512x512.png icon.png

writes `icons/icon.png`, `icons/x2/...`, `icons/icon.ico`, `icons/icon.icns`.

Then flip `bundle.active` to `true` in `src-tauri/tauri.conf.json`.

Clean-room note: the icon must be an original design — never derived from
PDF Gear, Nitro, Adobe, or Stirling-PDF artwork.