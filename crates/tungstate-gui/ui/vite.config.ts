import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

export default defineConfig({
  plugins: [vue()],
  // Tauri serves the built files from disk, so relative paths are required.
  base: "./",
  build: { target: "es2021", outDir: "dist", emptyOutDir: true },
  server: { port: 5173, strictPort: true },
  clearScreen: false,
});
