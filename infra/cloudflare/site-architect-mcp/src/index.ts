// Stateless, separately deployable OAuth MCP *resource* Worker.
// Authentication is delegated to a separately operated Authorization Server;
// no client-provided tenant, principal, role, URL or grant can cross this edge.
import {
  OAuthResourceServer,
  insufficientScope,
} from "@cloudflare/workers-oauth-provider";
import { McpServer } from "@modelcontextprotocol/server";
import { createMcpHandler, getMcpAuthContext } from "agents/mcp/server";
import { env } from "cloudflare:workers";
import { z } from "zod";
import { admitOneStep, validVerifiedSubject } from "./admission.js";

const WRITE_SCOPE = "site_architect.run_one";

// userId is set only by a signed-in and consented auth-server flow. The
// issuer must establish its provenance from an actual human IdP session.
type AuthProps = { userId: string };

// These opt-in bindings are intentionally omitted from the default (offline,
// non-public) config. The release gate must provision them before deployment;
// generated Env is inferred from cloudflare.config.ts by `cf workers types`.
const releaseEnv = env;

function createServer() {
  const server = new McpServer({ name: "siteArchitect", version: "0.1.0" });
  const authenticated = getMcpAuthContext()?.props as AuthProps | undefined;

  server.registerTool(
    "site_architect_run_one",
    {
      description: "Ask the private native operator to consume an already approved one-use SQL request. Never authorizes a new tenant, grant, source, budget or arbitrary group.",
      inputSchema: z.object({ request_id: z.uuid() }).strict(),
    },
    async ({ request_id }) => {
      const subject = authenticated && {
        issuer: releaseEnv.AUTH_SERVER_URL,
        subject: authenticated.userId,
        resource: releaseEnv.MCP_SERVER_URL,
      };
      const result = await admitOneStep(
        releaseEnv.ONE_STEP_ADMISSION,
        subject && validVerifiedSubject(subject) ? subject : undefined,
        request_id,
        releaseEnv.ADMISSION_SIGNING_KEY_HEX,
        releaseEnv.PRIVATE_ADMISSION_URL,
      );
      return {
        content: [{ type: "text" as const, text: JSON.stringify(result) }],
        isError: result.status !== "request_reserved",
      };
    },
  );
  return server;
}

const mcp = createMcpHandler(createServer, { legacy: "reject" });

export default new OAuthResourceServer<Env, AuthProps>({
  resourceMetadata: {
    resource: releaseEnv.MCP_SERVER_URL,
    authorization_servers: [releaseEnv.AUTH_SERVER_URL],
  },
  requiredScopes: [WRITE_SCOPE],
  validateToken: (runtimeEnv) => runtimeEnv.AUTH_SERVER.validateToken,
  handler: {
    fetch(request, runtimeEnv, ctx) {
      if (!ctx.auth.scope.includes(WRITE_SCOPE)) {
        return insufficientScope(ctx.auth, [WRITE_SCOPE]);
      }
      // Metadata is provided by OAuthResourceServer. Only the exact MCP path
      // is eligible for a tool request; this Worker has no public proxy.
      if (new URL(request.url).pathname !== "/mcp") {
        return new Response("Not found", { status: 404 });
      }
      return mcp(request, runtimeEnv, ctx);
    },
  },
});
