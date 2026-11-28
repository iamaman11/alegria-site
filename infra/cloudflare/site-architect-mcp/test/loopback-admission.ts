// Test-only bridge from the Worker's fixed service binding to a local Rust ingress.
import { readFile } from "node:fs/promises";
import {
  admitOneStep,
  sealOneStepAdmission,
  type AdmissionService,
  type SignedOneStepAdmissionV1,
  type VerifiedSubject,
} from "../src/admission.ts";

const [portText, requestId, keyFile, mode, ...extra] = process.argv.slice(2);
const port = Number(portText);
if (extra.length || !/^[1-9][0-9]{0,4}$/.test(portText ?? "") || port > 65535
  || !requestId || !keyFile || !mode || !["valid", "wrong-subject", "bad-signature", "expired"].includes(mode)) {
  throw new Error("invalid fixture arguments");
}

const signingKeyHex = await readFile(keyFile, "utf8");
if (!/^[0-9a-f]{64}$/.test(signingKeyHex)) throw new Error("invalid fixture key file");
const subject: VerifiedSubject = {
  issuer: "https://auth.example.test",
  resource: "https://mcp.example.test/mcp",
  subject: mode === "wrong-subject" ? "other-oauth-subject" : "ci-oauth-subject",
};

const service: AdmissionService = {
  async fetch(request) {
    if (request.url !== "http://admission.internal/v1/one-step" || request.method !== "POST"
      || request.headers.has("authorization")) throw new Error("unexpected admission request");
    let body = await request.text();
    if (mode === "expired") {
      body = JSON.stringify(await sealOneStepAdmission(subject, requestId, signingKeyHex, Math.floor(Date.now() / 1000) - 120));
    } else if (mode === "bad-signature") {
      const signed = JSON.parse(body) as SignedOneStepAdmissionV1;
      signed.signature_hex = (signed.signature_hex[0] === "0" ? "1" : "0") + signed.signature_hex.slice(1);
      body = JSON.stringify(signed);
    }
    return fetch(`http://127.0.0.1:${port}/v1/one-step`, {
      method: request.method,
      headers: request.headers,
      body,
      signal: request.signal,
      redirect: "error",
    });
  },
};

const reply = await admitOneStep(service, subject, requestId, signingKeyHex);
process.stdout.write(JSON.stringify(reply) + "\n");
