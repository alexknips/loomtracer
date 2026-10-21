import { createClient } from "@supabase/supabase-js";
import { publicSupabaseEnv } from "@/lib/env";
import { privateJson, requireSameOrigin } from "@/lib/server/http";

const MAX_BODY_BYTES = 4096;

async function readBody(request: Request): Promise<unknown> {
  const size = Number(request.headers.get("content-length") ?? 0);
  if (size > MAX_BODY_BYTES) throw new Error("TOO_LARGE");
  const reader = request.body?.getReader();
  if (!reader) throw new Error("INVALID_REQUEST");
  const chunks: Uint8Array[] = [];
  let total = 0;
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    total += value.byteLength;
    if (total > MAX_BODY_BYTES) {
      await reader.cancel();
      throw new Error("TOO_LARGE");
    }
    chunks.push(value);
  }
  const bytes = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.byteLength; }
  try { return JSON.parse(new TextDecoder().decode(bytes)); } catch { throw new Error("INVALID_REQUEST"); }
}

export async function POST(request: Request) {
  try {
    requireSameOrigin(request);
    if (request.headers.get("content-type")?.split(";")[0].trim().toLowerCase() !== "application/json") {
      return privateJson({ error: "Use a JSON request." }, { status: 415 });
    }
    const body = await readBody(request);
    if (!body || typeof body !== "object" || Array.isArray(body)) throw new Error("INVALID_REQUEST");
    const input = body as Record<string, unknown>;
    if (typeof input.email !== "string" || typeof input.use_case !== "string"
      || (input.website !== undefined && typeof input.website !== "string")) throw new Error("INVALID_REQUEST");
    const email = input.email.trim();
    const useCase = input.use_case.trim();
    const honeypot = typeof input.website === "string" ? input.website : "";
    if (email.length > 254 || !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email)
      || useCase.length < 10 || useCase.length > 1000 || honeypot.length > 1000) throw new Error("INVALID_REQUEST");
    // A fresh anonymous client deliberately carries no signed-in user's cookies.
    // The RPC owns validation, deduplication and throttling; no privileged key.
    const env = publicSupabaseEnv();
    const database = createClient(env.url, env.publishableKey, { auth: { persistSession: false, autoRefreshToken: false, detectSessionInUrl: false } });
    const { data, error } = await database.rpc("submit_access_request", { request_email: email, request_use_case: useCase, honeypot });
    if (error) {
      if (error.message.includes("RATE_LIMITED")) return privateJson({ error: "Please try again later." }, { status: 429, headers: { "Retry-After": "3600" } });
      if (error.message.includes("INVALID_REQUEST")) throw new Error("INVALID_REQUEST");
      return privateJson({ error: "We couldn't save your request. Please try again shortly." }, { status: 503 });
    }
    if (!data || data.ok !== true) return privateJson({ error: "We couldn't save your request. Please try again shortly." }, { status: 503 });
    return privateJson({ ok: true }, { status: 202 });
  } catch (error) {
    const message = error instanceof Error ? error.message : "";
    if (message === "FORBIDDEN") return privateJson({ error: "Access denied." }, { status: 403 });
    if (message === "TOO_LARGE") return privateJson({ error: "Request is too large." }, { status: 413 });
    if (message === "INVALID_REQUEST") return privateJson({ error: "Check your email and project description." }, { status: 400 });
    return privateJson({ error: "We couldn't save your request. Please try again shortly." }, { status: 503 });
  }
}
