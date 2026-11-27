// Pure admission transport boundary. Only an authenticated resource handler
// may call this, with identity taken from verified OAuth props, never JSON.

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const SAFE_ID = /^[\w.:-]{1,256}$/;

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

function reply(requestId: string, status: StartReply["status"], code: string): StartReply {
  return { status, code, request_id: requestId };
}

export function validRequestId(value: unknown): value is string {
  return typeof value === "string" && UUID.test(value) && value !== "00000000-0000-0000-0000-000000000000";
}

export function validVerifiedSubject(subject: VerifiedSubject): boolean {
  if (typeof subject.subject !== "string" || !SAFE_ID.test(subject.subject)
    || typeof subject.issuer !== "string" || typeof subject.resource !== "string") return false;
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

/** No temporal client, grant issuance, SQL connection or direct network URL. */
export async function admitOneStep(
  service: AdmissionService | undefined,
  subject: VerifiedSubject | undefined,
  requestId: string,
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
  // The URL selects an internal Service Binding endpoint. It is *not* fetched
  // with global fetch, is never supplied by the MCP caller and is not a tunnel.
  // Its backend must independently bind issuer+subject to SQL policy and consume
  // an existing one-use grant before the existing native Temporal operator.
  const body = JSON.stringify({
    schema_revision: 1,
    request_id: requestId,
    authenticated_user: subject,
  });
  try {
    const response = await service.fetch(new Request("https://admission.internal/v1/one-step", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body,
      signal: AbortSignal.timeout(10_000),
    }));
    if (!response.ok || Number(response.headers.get("content-length") ?? "0") > 2048) {
      return reply(requestId, "start_outcome_unknown", "admission_outcome_unconfirmed");
    }
    const text = await response.text();
    if (text.length > 2048) return reply(requestId, "start_outcome_unknown", "unattested_admission_reply");
    return parseAdmissionReply(JSON.parse(text) as unknown, requestId);
  } catch {
    return reply(requestId, "start_outcome_unknown", "admission_outcome_unconfirmed");
  }
}
