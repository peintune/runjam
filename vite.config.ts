import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import tailwindcss from "@tailwindcss/vite";
import { resolve } from "path";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// Detect build target: "landing" or default (Tauri app)
const buildTarget = process.env.BUILD_TARGET ?? "app";

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [vue(), tailwindcss()],

  resolve: {
    alias: {
      "@": resolve(__dirname, "src"),
    },
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: buildTarget === "landing" ? 5173 : 1420,
    strictPort: buildTarget !== "landing",
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },

  base: buildTarget === "landing" ? "./" : "/",

  build:
    buildTarget === "landing"
      ? {
          outDir: "dist-landing",
          emptyOutDir: true,
          rollupOptions: {
            input: {
              index: resolve(__dirname, "landing.html"),
            },
          },
        }
      : undefined,
}));
