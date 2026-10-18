import { resolve } from "node:path";
import { defineConfig } from "vite";

// Tauri expects a fixed dev port and the built UI in ./dist (see src-tauri/tauri.conf.json).
// One HTML page per window.
export default defineConfig({
  root: "ui",
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: {
    outDir: "../dist",
    emptyOutDir: true,
    target: "es2022",
    rollupOptions: {
      input: {
        main: resolve(import.meta.dirname, "ui/index.html"),
        equalizer: resolve(import.meta.dirname, "ui/equalizer.html"),
        playlist: resolve(import.meta.dirname, "ui/playlist.html"),
        openurl: resolve(import.meta.dirname, "ui/openurl.html"),
        fileinfo: resolve(import.meta.dirname, "ui/fileinfo.html"),
      },
    },
  },
});
