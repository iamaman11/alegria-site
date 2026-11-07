// Vite is only the official cf build adapter. It must not rewrite SEO HTML.
import { cloudflare } from "@cloudflare/vite-plugin";
import { defineConfig } from "vite";

const publicDir = process.env.ALEGRIA_CF_PUBLIC_DIR;
if (!publicDir) throw new Error("ALEGRIA_CF_PUBLIC_DIR is required");

export default defineConfig({
  publicDir,
  build: {
    rollupOptions: { input: "./build-entry.js" },
  },
  plugins: [cloudflare()],
});
