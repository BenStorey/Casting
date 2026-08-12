import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "node:path";

// The SPA builds to `dist/` (gitignored) and its contents are embedded into the
// Rust binary via rust-embed (see src/web.rs + build.rs).
//
// In dev, `npm run dev` serves the SPA from Vite on :5173 with hot-reload and
// proxies `/api` to the Rust `cast run` server, so you can iterate on the
// frontend without rebuilding or re-embedding. Start `cast run` first, then
// open http://127.0.0.1:5173.
export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
  server: {
    port: 5173,
    // Allow access via the public dev host (Caddy forwards it). Vite otherwise
    // blocks requests whose Host isn't localhost/127.0.0.1.
    allowedHosts: ["dev.benstorey.com"],
    proxy: {
      // Forward API + SSE to the `cast run` server (default 127.0.0.1:8080).
      "/api": {
        target: process.env.CAST_PROXY ?? "http://127.0.0.1:8080",
        changeOrigin: true,
        // SSE must not be buffered.
        configure: (proxy) => {
          proxy.on("proxyRes", (proxyRes) => {
            proxyRes.headers["Cache-Control"] = "no-cache";
          });
        },
      },
    },
  },
});
