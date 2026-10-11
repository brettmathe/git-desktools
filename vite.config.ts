import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// https://v2.tauri.app/start/frontend/vite/
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  // Desktop app: one bundle is fine (xterm alone is ~300 KB).
  build: { chunkSizeWarningLimit: 1500 },
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
});
