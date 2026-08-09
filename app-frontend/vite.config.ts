/// <reference types="vitest/config" />
import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// Tauri expects a fixed dev port to match tauri.conf.json's devUrl.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  // Vitest reuses this config (plugins, resolution) — see src/test/setup.ts.
  // jsdom everywhere: the pure-logic suites don't need it, but one shared
  // environment keeps the setup file (jest-dom + RTL cleanup) unconditional.
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    restoreMocks: true,
  },
});
