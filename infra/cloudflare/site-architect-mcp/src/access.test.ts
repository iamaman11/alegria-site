import assert from "node:assert/strict";
import test from "node:test";
import {
  verifiedAccessSubject,
  type AccessIdentity,
  type AccessPilotConfiguration,
  type VerifiedAccessContext,
} from "./access.ts";

const userId = "d54eb078-c5e9-49ba-bc6e-c3a91fa778ca";
const config = {
  aud: "piloted-access-app-aud",
  allowedEmail: "pilot@example.org",
  issuer: "https://pilot.cloudflareaccess.com",
  resource: "https://site-architect.example.org/mcp",
} satisfies AccessPilotConfiguration;

function context(identity: AccessIdentity | undefined, aud = config.aud): VerifiedAccessContext {
  return { aud: aud!, getIdentity: async () => identity };
}

test("directly authenticated Access identity pins the app, email, user and resource", async () => {
  assert.deepEqual(await verifiedAccessSubject(context({ email: "pilot@example.org", user_uuid: userId }), config), {
    issuer: config.issuer,
    resource: config.resource,
    subject: userId,
  });
});

test("missing Access, wrong app, wrong user or malformed issuer all fail closed", async () => {
  const validIdentity = { email: "pilot@example.org", user_uuid: userId };
  const cases: Array<[VerifiedAccessContext | undefined, AccessPilotConfiguration]> = [
    [undefined, config],
    [context(validIdentity, "other-app-aud"), config],
    [context(validIdentity), { allowedEmail: config.allowedEmail, issuer: config.issuer, resource: config.resource }],
    [context(validIdentity), { aud: config.aud, issuer: config.issuer, resource: config.resource }],
    [context(validIdentity), { ...config, allowedEmail: "PILOT@example.org" }],
    [context(validIdentity), { ...config, issuer: "http://pilot.cloudflareaccess.com" }],
    [context(validIdentity), { ...config, resource: "https://site-architect.example.org/other" }],
    [context({ ...validIdentity, email: "other@example.org" }), config],
    [context({ user_uuid: userId }), config],
    [context({ email: validIdentity.email }), config],
    [context({ ...validIdentity, user_uuid: "forged" }), config],
    [context(undefined), config],
    [{ aud: config.aud!, getIdentity: async () => { throw Error("Access unavailable"); } }, config],
  ];
  for (const [access, settings] of cases) {
    assert.equal(await verifiedAccessSubject(access, settings), undefined);
  }
});

test("untrusted request headers cannot replace Access identity", async () => {
  assert.equal(await verifiedAccessSubject(undefined, config), undefined);
  assert.equal(await verifiedAccessSubject(context({ email: "pilot@example.org" }), config), undefined);
});
