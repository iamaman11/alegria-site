// Separate resource Worker; never alter alegria-seo-staging or its routes.
// The default configuration has no public route. A release must explicitly
// provision a real OAuth issuer and the exact MCP resource audience.
import { bindings, defineConfig } from "cf/config";

const publicRelease = process.env.SITE_ARCHITECT_ENABLE_PUBLIC_MCP === "yes";
const resourceUrl = process.env.SITE_ARCHITECT_MCP_RESOURCE_URL;
const issuerUrl = process.env.SITE_ARCHITECT_OAUTH_ISSUER_URL;
const admissionWorker = process.env.SITE_ARCHITECT_PRIVATE_ADMISSION_WORKER;
const authWorker = process.env.SITE_ARCHITECT_AUTH_WORKER;

if (publicRelease && (!resourceUrl || !issuerUrl || !authWorker || !admissionWorker)) {
  throw new Error("Public MCP requires exact resource URL, OAuth issuer, auth Worker and private admission Worker");
}
if (publicRelease && (!resourceUrl?.startsWith("https://") || !issuerUrl?.startsWith("https://"))) {
  throw new Error("Public MCP OAuth endpoints must use HTTPS");
}

export default defineConfig({
  worker: {
    name: "alegria-site-architect-mcp",
    entrypoint: "./src/index.ts",
    compatibilityDate: "2026-11-27",
    compatibilityFlags: ["nodejs_compat"],
    workersDev: publicRelease,
    observability: { enabled: true, traces: { enabled: true } },
    env: {
      // Set only with Cloudflare's secret management; never committed as text.
      ADMISSION_SIGNING_KEY_HEX: bindings.secret(),
      AUTH_SERVER: authWorker ? bindings.worker({ worker: authWorker }) : undefined,
      AUTH_SERVER_URL: issuerUrl ? bindings.text(issuerUrl) : undefined,
      MCP_SERVER_URL: resourceUrl ? bindings.text(resourceUrl) : undefined,
      ONE_STEP_ADMISSION: admissionWorker
        ? bindings.worker({ worker: admissionWorker })
        : undefined,
    },
  },
});
