import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  build: {
    rollupOptions: {
      output: {
        manualChunks(id) {
          const normalizedId = id.replaceAll("\\", "/");
          if (normalizedId.endsWith("/src/i18n-en.ts")) return "i18n";
          if (normalizedId.includes("/node_modules/react/") || normalizedId.includes("/node_modules/react-dom/")) {
            return "react-vendor";
          }
          if (normalizedId.includes("/node_modules/@tauri-apps/")) return "tauri-vendor";
          if (
            normalizedId.includes("/node_modules/@dnd-kit/")
            || normalizedId.includes("/node_modules/@radix-ui/react-slot/")
            || normalizedId.includes("/node_modules/lucide-react/")
          ) {
            return "ui-vendor";
          }
          return undefined;
        },
      },
    },
  },
  server: {
    host: "127.0.0.1",
    port: 1420,
    strictPort: true,
  },
});
