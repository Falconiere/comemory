import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Built output is embedded into the `comemory` binary (see src/serve/assets.rs),
// so it must stay self-contained: relative-free absolute asset paths under
// `/assets/` that the axum fallback handler serves verbatim.
//
// For frontend iteration, run `comemory serve --port 8787` and `npm run dev`;
// the proxy forwards the JSON/file API to the running server.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  build: {
    outDir: "dist",
    assetsDir: "assets",
    emptyOutDir: true,
  },
  server: {
    proxy: {
      "/api": "http://127.0.0.1:8787",
    },
  },
});
