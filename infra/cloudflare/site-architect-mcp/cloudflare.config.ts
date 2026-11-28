// Separate resource Worker; never alter alegria-seo-staging or its routes.
// The default configuration has no public route. Release requires a separate
// Access-protected MCP application with Managed OAuth and an email Allow rule.
import { bindings, defineConfig } from "cf/config";

const publicRelease = process.env.SITE_ARCHITECT_ENABLE_PUBLIC_MCP === "yes";
// Account and technical hostname are canonical for this project. A release
// cannot silently inherit another Cloudflare account or point at alegria.by.
const technicalMcpResourceUrl = "https://alegria-site-architect-mcp.alegria-site.workers.dev/mcp";
const accessIssuerUrl = process.env.SITE_ARCHITECT_ACCESS_ISSUER_URL;
const accessAud = process.env.SITE_ARCHITECT_ACCESS_AUD;
const admissionVpcServiceId = process.env.SITE_ARCHITECT_PRIVATE_ADMISSION_VPC_SERVICE_ID;

if (publicRelease && (!accessIssuerUrl || !accessAud || !admissionVpcServiceId)) {
  throw new Error("Public MCP requires Access issuer and AUD, and private VPC Service");
}
if (publicRelease && !accessIssuerUrl?.startsWith("https://")) {
  throw new Error("Public MCP Access issuer must use HTTPS");
}

export default defineConfig({
  accountId: "78e5fcb3762c1473113d51838556da74",
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
      MCP_SERVER_URL: publicRelease ? bindings.text(technicalMcpResourceUrl) : undefined,
      ONE_STEP_ADMISSION: admissionVpcServiceId
        ? bindings.vpcService({ id: admissionVpcServiceId })
        : undefined,
    },
  },
});
