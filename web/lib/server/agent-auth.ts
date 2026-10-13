import { createClient, type SupabaseClient } from "@supabase/supabase-js";
import { isDemoMode, publicSupabaseEnv } from "@/lib/env";
import type { ActorRecord } from "@/types/graph";

export type AgentPermission = "context:read" | "entries:write";
export interface AgentContext {
  connectionId: string;
  userId: string;
  clientId: string;
  workspaceId: string;
  scopes: AgentPermission[];
  actor: ActorRecord;
  accessToken: string;
}
export const DEMO_AGENT_TOKEN = "demo-oauth-agent";
export function mcpResource() {
  const base = process.env.APP_URL?.replace(/\/$/, "");
  if (!base) throw new Error("APP_URL is required.");
  return `${base}/api/mcp`;
}
export function agentClient(accessToken: string): SupabaseClient {
  const env = publicSupabaseEnv();
  return createClient(env.url, env.publishableKey, {
    accessToken: async () => accessToken,
    auth: { persistSession: false, autoRefreshToken: false, detectSessionInUrl: false },
    global: { fetch: (input, init) => fetch(input, { ...init, cache: "no-store" }) },
  });
}
export function validAgentClaims(claims: Record<string, unknown>, issuer: string, resource: string, now = Date.now() / 1000) {
  const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
  return claims.iss === issuer && claims.role === "authenticated"
    && typeof claims.exp === "number" && claims.exp > now
    && [claims.sub, claims.session_id, claims.client_id, claims.nf_connection_id].every(v => typeof v === "string" && uuid.test(v))
    && (claims.aud === resource || (Array.isArray(claims.aud) && claims.aud.includes(resource)));
}
export async function authenticateAgent(request: Request): Promise<AgentContext | null> {
  const match = /^Bearer ([^\s]+)$/i.exec(request.headers.get("authorization") ?? "");
  if (!match) return null;
  const accessToken = match[1];
  if (isDemoMode() && accessToken === DEMO_AGENT_TOKEN) return {
    connectionId: "demo", userId: "owner", clientId: "demo", accessToken,
    workspaceId: "00000000-0000-0000-0000-000000000001", scopes: ["context:read", "entries:write"],
    actor: { id: "demo", kind: "agent", display_name: "Demo agent", platform: "test" },
  };
  if (accessToken.startsWith("nf_pat_") || accessToken.split(".").length !== 3) return null;
  try {
    const header = JSON.parse(Buffer.from(accessToken.split(".")[0], "base64url").toString("utf8"));
    if (!["ES256", "RS256"].includes(header.alg)) return null;
    const env = publicSupabaseEnv();
    // Verify with Supabase's signing keys; never accept decode-only claims.
    const verifier = createClient(env.url, env.publishableKey, { auth: { persistSession: false, autoRefreshToken: false } });
    const { data: verified, error } = await verifier.auth.getClaims(accessToken);
    if (error || !verified || !validAgentClaims(verified.claims, `${env.url}/auth/v1`, mcpResource())) return null;
    const { data, error: contextError } = await agentClient(accessToken).rpc("agent_request_context");
    if (contextError || !data || data.user_id !== verified.claims.sub || data.client_id !== verified.claims.client_id
      || data.connection_id !== verified.claims.nf_connection_id) return null;
    return { connectionId: data.connection_id, userId: data.user_id, clientId: data.client_id,
      workspaceId: data.workspace_id, scopes: data.permissions, actor: data.actor, accessToken };
  } catch { return null; }
}
