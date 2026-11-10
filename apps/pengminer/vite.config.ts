import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
export default defineConfig({
  plugins: [react()],
  base: "./",
  build: { target: "chrome61" },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/target/**", "**/src-tauri/binaries/**"],
    },
  },
  test: { environment: "jsdom", setupFiles: ["./src/test-setup.ts"] },
});
