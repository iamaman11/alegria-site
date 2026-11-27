// Separate resource Worker; never alter alegria-seo-staging or its routes.
// The default configuration has no public route. Release requires a separate
// Access-protected MCP application with Managed OAuth and an email Allow rule.
import { bindings, defineConfig } from "cf/config";

const publicRelease = process.env.SITE_ARCHITECT_ENABLE_PUBLIC_MCP === "yes";
const resourceUrl = process.env.SITE_ARCHITECT_MCP_RESOURCE_URL;
const accessIssuerUrl = process.env.SITE_ARCHITECT_ACCESS_ISSUER_URL;
const accessAud = process.env.SITE_ARCHITECT_ACCESS_AUD;
const admissionVpcServiceId = process.env.SITE_ARCHITECT_PRIVATE_ADMISSION_VPC_SERVICE_ID;
const admissionUrl = process.env.SITE_ARCHITECT_PRIVATE_ADMISSION_URL;

if (publicRelease && (!resourceUrl || !accessIssuerUrl || !accessAud || !admissionVpcServiceId || !admissionUrl)) {
  throw new Error("Public MCP requires exact resource URL, Access issuer and AUD, private VPC Service and HTTPS admission URL");
}
if (publicRelease && (!resourceUrl?.startsWith("https://") || !accessIssuerUrl?.startsWith("https://"))) {
  throw new Error("Public MCP Access issuer and resource must use HTTPS");
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
      ACCESS_ALLOWED_EMAIL: bindings.secret(),
      ACCESS_AUD: accessAud ? bindings.text(accessAud) : undefined,
      ACCESS_ISSUER_URL: accessIssuerUrl ? bindings.text(accessIssuerUrl) : undefined,
      MCP_SERVER_URL: resourceUrl ? bindings.text(resourceUrl) : undefined,
      PRIVATE_ADMISSION_URL: admissionUrl ? bindings.text(admissionUrl) : undefined,
      ONE_STEP_ADMISSION: admissionVpcServiceId
        ? bindings.vpcService({ id: admissionVpcServiceId })
        : undefined,
    },
  },
});
