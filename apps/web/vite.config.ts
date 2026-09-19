import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import path from "node:path";
import { defineConfig } from "vite-plus";

/** Where `ob serve` listens by default. The dev server proxies to it so that `pnpm dev` and the
 * built UI served by `ob serve --ui` fetch the identical relative paths — no `import.meta.env`
 * branch, and no CORS in the common case. Override with `OPEN_BROWSER_API` when the service runs
 * on another port. */
const API = process.env.OPEN_BROWSER_API ?? "http://127.0.0.1:8787";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: { "@": path.resolve(__dirname, "./src") },
  },
  server: {
    host: "127.0.0.1",
    port: 5173,
    strictPort: true,
    proxy: {
      // `ws: true` matters for /api/events; without it the upgrade request is proxied as plain
      // HTTP and the socket closes immediately with a 400.
      "/api": { target: API, changeOrigin: true, ws: true },
    },
  },
  build: {
    // Read by `ob serve --ui dist` and by the Tauri shell's `frontendDist`.
    outDir: "dist",
    emptyOutDir: true,
  },
  lint: {
    ignorePatterns: ["dist/**", "node_modules/**"],
    options: { typeAware: true },
  },
  fmt: {
    ignorePatterns: ["dist/**", "node_modules/**"],
  },
  test: {
    include: ["tests/**/*.test.ts"],
    environment: "node",
  },
  clearScreen: false,
});
