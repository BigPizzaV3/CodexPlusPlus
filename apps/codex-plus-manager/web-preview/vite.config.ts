import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";
import { defineConfig } from "vite";

const root = path.resolve(__dirname);

export default defineConfig({
  root,
  base: "./",
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: [
      { find: "@tauri-apps/api/core", replacement: path.resolve(root, "tauri-web-shim.ts") },
      { find: "@tauri-apps/api/event", replacement: path.resolve(root, "tauri-event-shim.ts") },
      { find: "@tauri-apps/plugin-dialog", replacement: path.resolve(root, "tauri-dialog-shim.ts") },
      { find: "@", replacement: path.resolve(root, "../src") },
    ],
  },
  build: {
    outDir: path.resolve(root, "../../../site/manager-preview"),
    emptyOutDir: true,
  },
  server: {
    host: "127.0.0.1",
    port: 1420,
    strictPort: true,
  },
});
