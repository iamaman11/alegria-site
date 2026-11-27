import assert from "node:assert/strict";
import test from "node:test";
import {
  admitOneStep,
  parseAdmissionReply,
  sealOneStepAdmission,
  signedAdmissionMessage,
  validPrivateAdmissionUrl,
  validRequestId,
  validVerifiedSubject,
  type AdmissionService,
  type SignedOneStepAdmissionV1,
  type VerifiedSubject,
} from "./admission.ts";

const requestId = "d54eb078-c5e9-49ba-bc6e-c3a91fa778ca";
const signingKeyHex = "ab".repeat(32);
const privateAdmissionUrl = "https://operator.private.example.org/v1/one-step";
const subject: VerifiedSubject = {
  issuer: "https://auth.example.org",
  subject: "verified-human-123",
  resource: "https://mcp.example.org/mcp",
};

test("reject malformed, nil or caller-shaped request IDs", () => {
  assert.equal(validRequestId(requestId), true);
  for (const input of ["", "00000000-0000-0000-0000-000000000000", "foo", "/etc/passwd", requestId.toUpperCase(), 22, null]) {
    assert.equal(validRequestId(input), false);
  }
});

test("verified subject must come from configured HTTPS resource, never an injected URL", () => {
  assert.equal(validVerifiedSubject(subject), true);
  for (const bad of [
    { ...subject, issuer: "http://auth.example.org" },
    { ...subject, subject: "tenant\nowner" },
    { ...subject, resource: "https://mcp.example.org/other" },
    { ...subject, resource: "https://mcp.example.org/mcp?admin=1" },
    { ...subject, resource: "https://admin:secret@mcp.example.org/mcp" },
  ]) assert.equal(validVerifiedSubject(bad), false);
  assert.equal(validVerifiedSubject({ ...subject, subject: 42 } as unknown as VerifiedSubject), false);
});

test("private admission URL is an exact operator-installed HTTPS endpoint", () => {
  assert.equal(validPrivateAdmissionUrl(privateAdmissionUrl), true);
  for (const value of [
    "http://operator.private.example.org/v1/one-step",
    "https://operator.private.example.org/other",
    "https://operator.private.example.org/v1/one-step?tenant=other",
    "https://operator.private.example.org/v1/one-step#fragment",
    "https://user:password@operator.private.example.org/v1/one-step",
    "https://operator.private.example.org/v1/one-step/",
    "https://operator.private.example.org:443/v1/one-step",
    "file:///v1/one-step",
    undefined,
  ]) assert.equal(validPrivateAdmissionUrl(value), false);
});

test("no private admission binding never attempts a Start", async () => {
  assert.deepEqual(await admitOneStep(undefined, subject, requestId, signingKeyHex), {
    status: "launch_blocked", code: "private_admission_not_configured", request_id: requestId,
  });
});

test("unverified identity and invalid IDs cannot reach a configured service", async () => {
  let calls = 0;
  const service: AdmissionService = { fetch: async () => { calls++; throw Error("must not be called"); } };
  assert.equal((await admitOneStep(service, undefined, requestId, signingKeyHex)).code, "verified_identity_unavailable");
  assert.equal((await admitOneStep(service, subject, "../../secrets", signingKeyHex)).code, "invalid_request_id");
  assert.equal((await admitOneStep(service, subject, requestId, undefined)).code, "private_admission_signing_key_not_configured");
  assert.equal((await admitOneStep(service, subject, requestId, signingKeyHex)).code, "private_admission_url_not_configured");
  assert.equal((await admitOneStep(service, subject, requestId, signingKeyHex, "https://evil.example.org/?url=private")).code, "private_admission_url_not_configured");
  assert.equal(calls, 0);
});

test("one-step attestation is scoped, 30-second HMAC-bound and cannot be modified", async () => {
  const signed = await sealOneStepAdmission(subject, requestId, signingKeyHex, 1_900_000_000);
  const bytes = Uint8Array.from(signingKeyHex.match(/../g)!, (pair) => Number.parseInt(pair, 16));
  const key = await crypto.subtle.importKey("raw", bytes, { name: "HMAC", hash: "SHA-256" }, false, ["verify"]);
  const decode = (hex: string) => Uint8Array.from(hex.match(/../g)!, (pair) => Number.parseInt(pair, 16));
  const unsigned = (({ signature_hex: _discard, ...fields }) => fields)(signed);
  assert.equal(signed.scope, "site_architect.run_one");
  assert.equal(signed.expires_at_unix_seconds - signed.issued_at_unix_seconds, 30);
  assert.equal(await crypto.subtle.verify("HMAC", key, decode(signed.signature_hex), new TextEncoder().encode(signedAdmissionMessage(unsigned))), true);
  assert.equal(await crypto.subtle.verify("HMAC", key, decode(signed.signature_hex), new TextEncoder().encode(signedAdmissionMessage({ ...unsigned, subject: "forged" }))), false);
  const fixed = { ...unsigned, nonce: "c62d0fe6-49a4-4b15-9ced-1dc3e0c6300d" };
  const staticMessage = signedAdmissionMessage({ ...fixed, issued_at_unix_seconds: 1_900_000_000, expires_at_unix_seconds: 1_900_000_030 });
  const staticMac = new Uint8Array(await crypto.subtle.sign("HMAC", await crypto.subtle.importKey("raw", bytes, { name: "HMAC", hash: "SHA-256" }, false, ["sign"]), new TextEncoder().encode(staticMessage)));
  assert.equal(Array.from(staticMac, (value) => value.toString(16).padStart(2, "0")).join(""), "c02cd955c775580df128b99f73873c1443b19575b8f4e552f5b53e4d1d70eaea");
  await assert.rejects(sealOneStepAdmission(subject, requestId, "bad-secret", 1_900_000_000));
});

test("private handoff carries signed verified subject and opaque ID; never a bearer token", async () => {
  let calls = 0;
  const service: AdmissionService = { fetch: async (request) => {
    calls++;
    assert.equal(request.url, privateAdmissionUrl);
    assert.equal(request.method, "POST");
    assert.equal(request.headers.has("authorization"), false);
    const body = await request.json() as SignedOneStepAdmissionV1;
    assert.equal(body.schema_revision, 1);
    assert.equal(body.request_id, requestId);
    assert.equal(body.issuer, subject.issuer);
    assert.equal(body.audience, subject.resource);
    assert.equal(body.subject, subject.subject);
    assert.match(body.signature_hex, /^[0-9a-f]{64}$/);
    assert.match(body.nonce, /^[0-9a-f-]{36}$/);
    assert.equal(body.expires_at_unix_seconds - body.issued_at_unix_seconds, 30);
    assert.deepEqual(Object.keys(body), ["schema_revision", "request_id", "issuer", "audience", "subject", "scope", "issued_at_unix_seconds", "expires_at_unix_seconds", "nonce", "signature_hex"]);
    return Response.json({ schema_revision: 1, request_id: requestId, status: "request_reserved", code: "one_use_reserved" });
  } };
  assert.deepEqual(await admitOneStep(service, subject, requestId, signingKeyHex, privateAdmissionUrl), {
    status: "request_reserved", code: "one_use_reserved", request_id: requestId,
  });
  assert.equal(calls, 1);
});

test("transport failures, non-2xx and malformed replies never become success or retries", async () => {
  const cases: AdmissionService[] = [
    { fetch: async () => { throw Error("connection reset after grant reservation"); } },
    { fetch: async () => Response.json({ code: "forbidden" }, { status: 403 }) },
    { fetch: async () => Response.json({ schema_revision: 1, request_id: requestId, status: "started", code: "done" }) },
    { fetch: async () => Response.json({ schema_revision: 1, request_id: "different", status: "request_reserved", code: "wrong" }) },
    { fetch: async () => Response.json({ schema_revision: 1, request_id: requestId, status: "request_reserved", code: "ok", provider_token: "secret" }) },
    { fetch: async () => Response.json({ schema_revision: 1, request_id: requestId, status: ["request_reserved"], code: "ok" }) },
    { fetch: async () => new Response("x".repeat(4096), { headers: { "content-type": "application/json" } }) },
  ];
  for (const service of cases) {
    assert.equal((await admitOneStep(service, subject, requestId, signingKeyHex, privateAdmissionUrl)).status, "start_outcome_unknown");
  }
});

test("sealed reply never confuses an accepted reservation with Temporal History", () => {
  assert.equal(parseAdmissionReply({schema_revision:1,request_id:requestId,status:"request_reserved",code:"reserved"},requestId).status,"request_reserved");
  assert.equal(parseAdmissionReply({schema_revision:1,request_id:requestId,status:"complete",code:"success"},requestId).status,"start_outcome_unknown");
});
