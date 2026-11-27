import assert from "node:assert/strict";
import test from "node:test";
import {
  admitOneStep,
  parseAdmissionReply,
  validRequestId,
  validVerifiedSubject,
  type AdmissionService,
  type VerifiedSubject,
} from "./admission.ts";

const requestId = "d54eb078-c5e9-49ba-bc6e-c3a91fa778ca";
const subject: VerifiedSubject = {
  issuer: "https://auth.example.org",
  subject: "verified-human-123",
  resource: "https://mcp.example.org/mcp",
};

test("reject malformed, nil or caller-shaped request IDs", () => {
  assert.equal(validRequestId(requestId), true);
  for (const input of ["", "00000000-0000-0000-0000-000000000000", "foo", "/etc/passwd", 22, null]) {
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

test("no private admission binding never attempts a Start", async () => {
  assert.deepEqual(await admitOneStep(undefined, subject, requestId), {
    status: "launch_blocked", code: "private_admission_not_configured", request_id: requestId,
  });
});

test("unverified identity and invalid IDs cannot reach a configured service", async () => {
  let calls = 0;
  const service: AdmissionService = { fetch: async () => { calls++; throw Error("must not be called"); } };
  assert.equal((await admitOneStep(service, undefined, requestId)).code, "verified_identity_unavailable");
  assert.equal((await admitOneStep(service, subject, "../../secrets")).code, "invalid_request_id");
  assert.equal(calls, 0);
});

test("private handoff carries only verified subject and opaque ID; never a bearer token", async () => {
  let calls = 0;
  const service: AdmissionService = { fetch: async (request) => {
    calls++;
    assert.equal(request.url, "https://admission.internal/v1/one-step");
    assert.equal(request.method, "POST");
    assert.equal(request.headers.has("authorization"), false);
    assert.deepEqual(await request.json(), {
      schema_revision: 1, request_id: requestId, authenticated_user: subject,
    });
    return Response.json({ schema_revision: 1, request_id: requestId, status: "request_reserved", code: "one_use_reserved" });
  } };
  assert.deepEqual(await admitOneStep(service, subject, requestId), {
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
  ];
  for (const service of cases) {
    assert.equal((await admitOneStep(service, subject, requestId)).status, "start_outcome_unknown");
  }
});

test("sealed reply never confuses an accepted reservation with Temporal History", () => {
  assert.equal(parseAdmissionReply({schema_revision:1,request_id:requestId,status:"request_reserved",code:"reserved"},requestId).status,"request_reserved");
  assert.equal(parseAdmissionReply({schema_revision:1,request_id:requestId,status:"complete",code:"success"},requestId).status,"start_outcome_unknown");
});
