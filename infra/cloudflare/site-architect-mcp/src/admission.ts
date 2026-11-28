// Pure admission transport boundary. Only an authenticated resource handler
// may call this, with identity taken from verified OAuth props, never JSON.

// Native Rust verifies canonical lowercase UUID bytes before consuming a
// single-use SQL grant. Reject alternative spellings at the public edge too.
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const SAFE_ID = /^[\w.:-]{1,256}$/;
const SIGNING_KEY_HEX = /^[0-9a-f]{64}$/;
const EXACT_SCOPE = "site_architect.run_one";
const PRIVATE_ADMISSION_URL = "http://admission.internal/v1/one-step";
const ATTESTATION_TTL_SECONDS = 30;
const MAX_ADMISSION_REPLY_BYTES = 2048;

export type StartReply = {
  status: "launch_blocked" | "request_reserved" | "start_outcome_unknown";
  code: string;
  request_id: string;
};

export interface AdmissionService {
  fetch(request: Request): Promise<Response>;
}

export interface VerifiedSubject {
  issuer: string;
  subject: string;
  resource: string;
}

/** Wire fields are sealed in this exact order, independently of JSON key order. */
export interface SignedOneStepAdmissionV1 {
  schema_revision: 1;
  request_id: string;
  issuer: string;
  audience: string;
  subject: string;
  scope: typeof EXACT_SCOPE;
  issued_at_unix_seconds: number;
  expires_at_unix_seconds: number;
  nonce: string;
  signature_hex: string;
}

export function signedAdmissionMessage(input: Omit<SignedOneStepAdmissionV1, "signature_hex">): string {
  return [
    "site-architect-admission-v1", input.request_id, input.issuer,
    input.audience, input.subject, input.scope,
    input.issued_at_unix_seconds, input.expires_at_unix_seconds, input.nonce, "",
  ].join("\n");
}

export async function sealOneStepAdmission(
  subject: VerifiedSubject,
  requestId: string,
  signingKeyHex: string,
  issuedAtUnixSeconds: number,
): Promise<SignedOneStepAdmissionV1> {
  if (!validRequestId(requestId) || !validVerifiedSubject(subject)
    || !SIGNING_KEY_HEX.test(signingKeyHex)
    || !Number.isSafeInteger(issuedAtUnixSeconds) || issuedAtUnixSeconds <= 0) {
    throw new Error("untrusted admission signing inputs");
  }
  const bytes = new Uint8Array(32);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = Number.parseInt(signingKeyHex.slice(i * 2, i * 2 + 2), 16);
  }
  const unsigned: Omit<SignedOneStepAdmissionV1, "signature_hex"> = {
    schema_revision: 1 as const,
    request_id: requestId,
    issuer: subject.issuer,
    audience: subject.resource,
    subject: subject.subject,
    scope: EXACT_SCOPE,
    issued_at_unix_seconds: issuedAtUnixSeconds,
    expires_at_unix_seconds: issuedAtUnixSeconds + ATTESTATION_TTL_SECONDS,
    nonce: crypto.randomUUID(),
  };
  const key = await crypto.subtle.importKey("raw", bytes, { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
  const signature = new Uint8Array(await crypto.subtle.sign("HMAC", key, new TextEncoder().encode(signedAdmissionMessage(unsigned))));
  const signature_hex = Array.from(signature, (byte) => byte.toString(16).padStart(2, "0")).join("");
  return { ...unsigned, signature_hex };
}

function reply(requestId: string, status: StartReply["status"], code: string): StartReply {
  return { status, code, request_id: requestId };
}

export function validRequestId(value: unknown): value is string {
  return typeof value === "string" && UUID.test(value) && value !== "00000000-0000-0000-0000-000000000000";
}

export function validVerifiedSubject(subject: VerifiedSubject): boolean {
  if (typeof subject.subject !== "string" || !SAFE_ID.test(subject.subject)
    || typeof subject.issuer !== "string" || typeof subject.resource !== "string") return false;
  if ([subject.issuer, subject.resource].some((value) => value.length > 512
    || !/^[\x21-\x7e]+$/.test(value) || /[@?#]/.test(value))) return false;
  try {
    const issuer = new URL(subject.issuer);
    const resource = new URL(subject.resource);
    return issuer.protocol === "https:" && resource.protocol === "https:"
      && !issuer.username && !issuer.password && !resource.username && !resource.password
      && resource.pathname === "/mcp" && !resource.search && !resource.hash
      && !issuer.search && !issuer.hash;
  } catch {
    return false;
  }
}

// An admission response must be a small, sealed DTO. Unknown/error responses
// are always ambiguous once a request was sent: never automatically replay.
export function parseAdmissionReply(raw: unknown, requestId: string): StartReply {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) {
    return reply(requestId, "start_outcome_unknown", "unattested_admission_reply");
  }
  const row = raw as Record<string, unknown>;
  if (Object.keys(row).some((key) => !["schema_revision", "request_id", "status", "code"].includes(key))
    || row.schema_revision !== 1 || row.request_id !== requestId
    || typeof row.status !== "string"
    || !["launch_blocked", "request_reserved", "start_outcome_unknown"].includes(row.status)
    || typeof row.code !== "string" || !SAFE_ID.test(row.code)) {
    return reply(requestId, "start_outcome_unknown", "unattested_admission_reply");
  }
  return reply(requestId, row.status as StartReply["status"], row.code);
}

// A private service can fail or be misconfigured. Never buffer an unbounded
// response in a public-facing Worker, even when Content-Length is missing.
async function readBoundedAdmissionReply(response: Response): Promise<string | undefined> {
  const reader = response.body?.getReader();
  if (!reader) return undefined;
  const decoder = new TextDecoder("utf-8", { fatal: true });
  let total = 0;
  let body = "";
  while (true) {
    const { done, value } = await reader.read();
    if (done) return body + decoder.decode();
    total += value.byteLength;
    if (total > MAX_ADMISSION_REPLY_BYTES) {
      await reader.cancel();
      return undefined;
    }
    body += decoder.decode(value, { stream: true });
  }
}

/** No temporal client, grant issuance, SQL connection or direct network URL. */
export async function admitOneStep(
  service: AdmissionService | undefined,
  subject: VerifiedSubject | undefined,
  requestId: string,
  signingKeyHex: string | undefined,
): Promise<StartReply> {
  if (!validRequestId(requestId)) {
    return reply(requestId, "launch_blocked", "invalid_request_id");
  }
  if (!subject || !validVerifiedSubject(subject)) {
    return reply(requestId, "launch_blocked", "verified_identity_unavailable");
  }
  if (!service) {
    return reply(requestId, "launch_blocked", "private_admission_not_configured");
  }
  if (!signingKeyHex || !SIGNING_KEY_HEX.test(signingKeyHex)) {
    return reply(requestId, "launch_blocked", "private_admission_signing_key_not_configured");
  }
  // The installed VPC Service binding pins the private host and port. Its
  // fixed request URL is only an internal Host header, never a global fetch
  // destination or a public proxy. The tunnel encrypts the remote leg; a
  // local cloudflared must share loopback with the native HTTP ingress.
  // The native ingress must independently bind issuer+subject to SQL policy and consume
  // an existing one-use grant before the existing native Temporal operator.
  let body: string;
  try {
    body = JSON.stringify(await sealOneStepAdmission(
      subject, requestId, signingKeyHex, Math.floor(Date.now() / 1000),
    ));
  } catch {
    return reply(requestId, "launch_blocked", "admission_signature_unavailable");
  }
  try {
    const response = await service.fetch(new Request(PRIVATE_ADMISSION_URL, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body,
      signal: AbortSignal.timeout(10_000),
    }));
    if (!response.ok || Number(response.headers.get("content-length") ?? "0") > MAX_ADMISSION_REPLY_BYTES) {
      return reply(requestId, "start_outcome_unknown", "admission_outcome_unconfirmed");
    }
    const text = await readBoundedAdmissionReply(response);
    if (text === undefined) return reply(requestId, "start_outcome_unknown", "unattested_admission_reply");
    return parseAdmissionReply(JSON.parse(text) as unknown, requestId);
  } catch {
    return reply(requestId, "start_outcome_unknown", "admission_outcome_unconfirmed");
  }
}
