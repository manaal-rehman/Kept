import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// The UI lives in ./ui and is bundled into ./dist, which Tauri embeds into the binary.
export default defineConfig({
  root: "ui",
  plugins: [react()],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    outDir: "../dist",
    emptyOutDir: true,
    target: "es2021",
    minify: "esbuild",
    sourcemap: false,
    chunkSizeWarningLimit: 600,
  },
});
