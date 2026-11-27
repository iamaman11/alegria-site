import { validVerifiedSubject, type VerifiedSubject } from "./admission.ts";

// Access has already authenticated the directly invoked Worker request. Only
// the two verified fields needed for this one-user pilot cross the boundary.
export interface AccessIdentity {
  email?: string;
  user_uuid?: string;
}

export interface VerifiedAccessContext {
  aud: string;
  getIdentity(): Promise<AccessIdentity | undefined>;
}

export interface AccessPilotConfiguration {
  aud?: string;
  allowedEmail?: string;
  issuer?: string;
  resource?: string;
}

const ACCESS_USER_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const EMAIL = /^[a-z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-z0-9](?:[a-z0-9.-]*[a-z0-9])?$/;

/** Never trust a request header, model argument, or an unpinned Access app. */
export async function verifiedAccessSubject(
  access: VerifiedAccessContext | undefined,
  config: AccessPilotConfiguration,
): Promise<VerifiedSubject | undefined> {
  if (!access || !config.aud || access.aud !== config.aud
    || !config.allowedEmail || config.allowedEmail.length > 254
    || !EMAIL.test(config.allowedEmail)) return undefined;
  let identity: AccessIdentity | undefined;
  try {
    identity = await access.getIdentity();
  } catch {
    return undefined;
  }
  if (!identity || identity.email !== config.allowedEmail
    || typeof identity.user_uuid !== "string" || !ACCESS_USER_ID.test(identity.user_uuid)) {
    return undefined;
  }
  const subject = {
    issuer: config.issuer ?? "",
    resource: config.resource ?? "",
    subject: identity.user_uuid,
  };
  return validVerifiedSubject(subject) ? subject : undefined;
}
