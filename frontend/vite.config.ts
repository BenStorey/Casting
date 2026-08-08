import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// The SPA builds to `dist/` (already .gitignore'd) whose contents are embedded
// into the Rust binary via rust-embed. No need to write assets to the server.
export default defineConfig({
  plugins: [react()],
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
});
