import path from "node:path";
import { fileURLToPath } from "node:url";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

const dirname = path.dirname(fileURLToPath(import.meta.url));

export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    host: "127.0.0.1",
    // memorph dev frontend; sits next to the API on 3223 — avoids Vite's default 5173
    port: 3224,
    strictPort: true,
    proxy: {
      "/api": process.env.MEMORPH_API_TARGET ?? "http://127.0.0.1:3223",
    },
  },
  resolve: {
    alias: {
      "@": path.resolve(dirname, "./src"),
    },
  },
});
