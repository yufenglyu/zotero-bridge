import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

export default defineConfig({
  plugins: [vue()],
  // Relative paths so the Tauri webview can load dist assets.
  base: "./",
  clearScreen: false,
  server: {
    strictPort: false,
  },
  build: {
    outDir: "dist",
    target: "es2021",
  },
});
