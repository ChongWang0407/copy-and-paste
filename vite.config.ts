import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react()],

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
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
  build: {
    outDir: "dist/web",
    emptyOutDir: true,
    // Smaller production assets for embedding into the Tauri binary
    target: "es2021",
    minify: "esbuild",
    cssMinify: true,
    sourcemap: false,
    reportCompressedSize: false,
    // Keep one main chunk; Tauri loads from disk, not network
    assetsInlineLimit: 4096,
    // The copy capsule is its own small window with its own page.
    rollupOptions: {
      input: {
        main: "index.html",
        capsule: "capsule.html",
      },
    },
  },
  test: {
    include: ["src/**/*.{test,spec}.{js,mjs,cjs,ts,mts,cts,jsx,tsx}"],
    exclude: ["**/node_modules/**", "**/dist/**", "**/artifacts/**"],
    // `npm run test:coverage`. Report only, no thresholds yet: the point is to
    // see where the gaps are. Files no test imports count as 0% (vitest default).
    coverage: {
      provider: "v8",
      include: ["src/**/*.{ts,tsx}"],
      // Data tables, not logic.
      exclude: ["src/**/*.test.{ts,tsx}", "src/**/*.d.ts", "src/locales.ts"],
      reporter: ["text-summary", "html", "json-summary"],
      reportsDirectory: "coverage",
    },
  },
}));
