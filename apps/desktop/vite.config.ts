import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// Tauri serves the dev server and embeds `dist/` in the release binary.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    // A silent fallback to another port would leave Tauri loading nothing.
    strictPort: true,
  },
  build: {
    // Match the WebView Tauri ships against rather than the browser default.
    target: "safari15",
    sourcemap: true,
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
    include: ["src/**/*.test.{ts,tsx}"],
  },
});
