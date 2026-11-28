import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import test from "node:test";
import { convertV4MiniflareOptions, Miniflare } from "miniflare";

const origin = "https://mcp.example.test";
const allowedAudience = "fixture-app";
const allowedEmail = "fixture@example.test";
const older = "2025-06-18";
const modern = "2026-07-28";

// Only this outer test module supplies the simulated Access context.
// The imported Worker is the exact bundle produced by the dry run.
const fixture = `
import worker from "./pilot.mjs";
export default {
  fetch(request, env, ctx) {
    const aud = request.headers.get("X-Fixture-Audience");
    if (!aud) return worker.fetch(request, env, ctx);
    return worker.fetch(request, env, {
      waitUntil: ctx.waitUntil.bind(ctx),
      passThroughOnException: ctx.passThroughOnException.bind(ctx),
      access: {
        aud,
        async getIdentity() {
          return { email: env.ACCESS_ALLOWED_EMAIL, user_uuid: "32085907-1ada-4de0-89da-642cb58cbcba" };
        },
      },
    });
  },
};
`;

async function rpcResult(response) {
  const body = await response.text();
  assert.equal(response.status, 200, body);
  const data = response.headers.get("content-type")?.includes("text/event-stream")
    ? body.split("\n").find((line) => line.startsWith("data: "))?.slice(6)
    : body;
  const message = JSON.parse(data);
  assert.equal(message.error, undefined);
  return message.result;
}

test("the Worker serves both stateless MCP versions behind the same Access gate", async () => {
  const bundle = await readFile(
    new URL("../.cloudflare/output/v1/workers/default/bundle/index.js", import.meta.url),
    "utf8",
  );
  const runtime = new Miniflare(convertV4MiniflareOptions({ workers: [{
    name: "fixture",
    modules: [
      { type: "ESModule", path: resolve("fixture.mjs"), contents: fixture },
      { type: "ESModule", path: resolve("pilot.mjs"), contents: bundle },
    ],
    compatibilityDate: "2026-11-27",
    compatibilityFlags: ["nodejs_compat"],
    bindings: {
      MCP_SERVER_URL: origin + "/mcp",
      ACCESS_AUD: allowedAudience,
      ACCESS_ISSUER_URL: "https://auth.example.test",
      ACCESS_ALLOWED_EMAIL: allowedEmail,
    },
  }] }));
  try {
    function send(path, audience, protocol, method, params = {}) {
      const headers = {
        "content-type": "application/json",
        accept: "application/json, text/event-stream",
        "mcp-protocol-version": protocol,
        "mcp-method": method,
        "cf-access-authenticated-user-email": allowedEmail,
      };
      if (audience) headers["x-fixture-audience"] = audience;
      if (protocol === modern) {
        if (method === "tools/call") headers["mcp-name"] = params.name;
        params = {
          ...params,
          _meta: {
            "io.modelcontextprotocol/protocolVersion": modern,
            "io.modelcontextprotocol/clientCapabilities": {},
          },
        };
      }
      return runtime.dispatchFetch(origin + path, {
        method: "POST",
        headers,
        body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }),
      });
    }

    assert.equal((await send("/other", allowedAudience, older, "tools/list")).status, 404);
    const initialized = await rpcResult(await send("/mcp", allowedAudience, older, "initialize", {
      protocolVersion: older, capabilities: {}, clientInfo: { name: "fixture", version: "1.0.0" },
    }));
    assert.equal(initialized.protocolVersion, older);
    for (const protocol of [older, modern]) {
      assert.equal((await send("/mcp", undefined, protocol, "tools/list")).status, 403);
      assert.equal((await send("/mcp", "other-app", protocol, "tools/list")).status, 403);
      const listed = await rpcResult(await send("/mcp", allowedAudience, protocol, "tools/list"));
      assert.deepEqual(listed.tools.map((tool) => tool.name), ["site_architect_run_one"]);
      const called = await rpcResult(await send("/mcp", allowedAudience, protocol, "tools/call", {
        name: "site_architect_run_one",
        arguments: { request_id: "d54eb078-c5e9-49ba-bc6e-c3a91fa778ca" },
      }));
      assert.equal(JSON.parse(called.content[0].text).code, "private_admission_not_configured");
    }
  } finally {
    await runtime.dispose();
  }
});
