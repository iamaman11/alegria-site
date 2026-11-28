// One-tool MCP behind Cloudflare Access Managed OAuth. Access handles human
// sign-in; this Worker independently pins app audience and verified user.
import { McpServer } from "@modelcontextprotocol/server";
import { createMcpHandler } from "agents/mcp/server";
import { z } from "zod";
import { admitOneStep, type VerifiedSubject } from "./admission.js";
import { verifiedAccessSubject } from "./access.js";

// These opt-in bindings are intentionally omitted from the default (offline,
// non-public) config. The release gate must provision them before deployment;
// generated Env is inferred from cloudflare.config.ts by `cf workers types`.
function createServer(subject: VerifiedSubject, releaseEnv: Env) {
  const server = new McpServer({ name: "siteArchitect", version: "0.1.0" });

  server.registerTool(
    "site_architect_run_one",
    {
      description: "Ask the private native operator to consume an already approved one-use SQL request. Never authorizes a new tenant, grant, source, budget or arbitrary group.",
      inputSchema: z.object({ request_id: z.uuid() }).strict(),
    },
    async ({ request_id }) => {
      const result = await admitOneStep(
        releaseEnv.ONE_STEP_ADMISSION,
        subject,
        request_id,
        releaseEnv.ADMISSION_SIGNING_KEY_HEX,
      );
      return {
        content: [{ type: "text" as const, text: JSON.stringify(result) }],
        isError: result.status !== "request_reserved",
      };
    },
  );
  return server;
}

export default {
  async fetch(request: Request, runtimeEnv: Env, ctx: ExecutionContext): Promise<Response> {
    // Managed OAuth is enabled on the single Access application covering this
    // exact URL. Never accept the same signed-in user on another resource URL.
    const url = new URL(request.url);
    if (`${url.origin}${url.pathname}` !== runtimeEnv.MCP_SERVER_URL) {
      return new Response("Not found", { status: 404 });
    }
    const subject = await verifiedAccessSubject(ctx.access, {
      aud: runtimeEnv.ACCESS_AUD,
      allowedEmail: runtimeEnv.ACCESS_ALLOWED_EMAIL,
      issuer: runtimeEnv.ACCESS_ISSUER_URL,
      resource: runtimeEnv.MCP_SERVER_URL,
    });
    if (!subject) {
      return new Response("Access identity required", { status: 403 });
    }
    // Request-local factory: no mutable global identity and no second OAuth
    // server. Access app policy grants only this single mutating MCP tool;
    // the native operator still requires its own signed one-use SQL grant.
    const mcp = createMcpHandler(() => createServer(subject, runtimeEnv), { legacy: "reject" });
    return mcp(request, runtimeEnv, ctx);
  },
};
