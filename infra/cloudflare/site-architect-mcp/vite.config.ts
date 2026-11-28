// cf CLI's native Worker's build adapter; no HTML/static assets or deployment.
import { cloudflare } from "@cloudflare/vite-plugin";
import { defineConfig } from "vite";

export default defineConfig({ plugins: [cloudflare()] });
