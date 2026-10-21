import { z } from "zod";
import { privateJson } from "./http";

export const projectRefSchema = z.string().regex(/^project:[a-z0-9][a-z0-9._/-]*$/).max(160);
export const operationSchema = z.string().min(8).max(160);
export const headSchema = z.string().min(1).nullable();
export const HOSTED_BUNDLE_LIMIT = 4 * 1024 * 1024;

/** Enforce the limit while reading; Content-Length alone is not trustworthy. */
export async function readReasoningJson(request: Request): Promise<unknown> {
  if (!request.headers.get("content-type")?.toLowerCase().includes("application/json")) throw new Error("JSON_REQUIRED");
  const reader = request.body?.getReader();
  if (!reader) throw new Error("EMPTY_BODY");
  const chunks: Uint8Array[] = []; let size = 0;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.byteLength;
      if (size > HOSTED_BUNDLE_LIMIT) { await reader.cancel(); throw new Error("BUNDLE_TOO_LARGE"); }
      chunks.push(value);
    }
  } finally { reader.releaseLock(); }
  return JSON.parse(Buffer.concat(chunks).toString("utf8"));
}

export function reasoningError(error: unknown) {
  const message = error instanceof Error ? error.message : "Unexpected error";
  if (message.includes("UNAUTHORIZED")) return privateJson({ error: "Sign in to access a hosted project." }, { status: 401 });
  if (/MAINTENANCE|WRITE_FREEZE/.test(message)) return privateJson({ error: "Loomtracer is undergoing maintenance. Your local file remains available.", code: "MAINTENANCE" }, { status: 503, headers: { "Retry-After": "60" } });
  if (/STALE|CONFLICT|IMMUTABLE|OPERATION_REUSED|operation id reused|history cannot be removed/i.test(message)) return privateJson({ error: "The project changed or this record already exists with different content. Reload before saving; your local changes have been kept.", code: "CONFLICT" }, { status: 409 });
  if (/FORBIDDEN|access denied|permission denied|AUTHORITY_REQUIRED|owner access required|commitment authority required/i.test(message)) return privateJson({ error: "You do not have permission for this action." }, { status: 403 });
  if (message.includes("BUNDLE_TOO_LARGE")) return privateJson({ error: "Hosted saves support bundles up to 4 MB. Export this file to keep it locally." }, { status: 413 });
  if (/not[ _]found/i.test(message)) return privateJson({ error: "The project or dataset was not found." }, { status: 404 });
  return privateJson({ error: "The reasoning request is invalid. Check the bundle and selected project." }, { status: 400 });
}
