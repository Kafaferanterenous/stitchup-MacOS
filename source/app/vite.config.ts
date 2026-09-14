import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { readFileSync } from "node:fs";

// Tauri expects a fixed dev server port and no clearing when running under the
// webview. `tauri.conf.json` points here for the dev URL.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  // pdf.js worker is loaded from node_modules as an asset, not bundled.
  optimizeDeps: { exclude: [] },
  build: {
    target: "es2022",
  },
  define: {
    // Expose the crate version for About -> Licenses without an extra request.
    __STITCHUP_CORE_VERSION__: JSON.stringify(
      (() => {
        try {
          const toml = readFileSync("../core/Cargo.toml", "utf8");
          return (toml.match(/^version\s*=\s*"([^"]+)"/m) ?? [])[1] ?? "dev";
        } catch {
          return "dev";
        }
      })(),
    ),
  },
});