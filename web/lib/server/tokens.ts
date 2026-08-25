import { createHash, randomBytes, randomUUID, timingSafeEqual } from "node:crypto";
import { createAdminClient } from "@/lib/supabase/admin";
import { isDemoMode } from "@/lib/env";
import type { ActorRecord } from "@/types/graph";

export const TOKEN_SCOPES = ["context:read", "entries:write"] as const;
export const DEMO_PAT = `nf_pat_00000000-0000-4000-8000-000000000002.${"A".repeat(43)}`;
export type TokenScope = typeof TOKEN_SCOPES[number];

export interface PatContext {
  tokenId: string;
  workspaceId: string;
  scopes: TokenScope[];
  actor: ActorRecord;
}

export function hashTokenSecret(secret: string): string {
  return createHash("sha256").update(secret, "utf8").digest("hex");
}

export function generatePat(): { tokenId: string; secret: string; token: string; prefix: string; secretHash: string } {
  const tokenId = randomUUID();
  const secret = randomBytes(32).toString("base64url");
  const prefix = `nf_pat_${tokenId}`;
  return { tokenId, secret, token: `${prefix}.${secret}`, prefix, secretHash: hashTokenSecret(secret) };
}

export function parsePat(token: string): { tokenId: string; secret: string } | null {
  const match = /^nf_pat_([0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12})\.([A-Za-z0-9_-]{43})$/i.exec(token);
  return match ? { tokenId: match[1].toLowerCase(), secret: match[2] } : null;
}

export function secretMatches(secret: string, expectedHash: string): boolean {
  const actual = Buffer.from(hashTokenSecret(secret), "hex");
  const expected = Buffer.from(expectedHash, "hex");
  return actual.length === expected.length && timingSafeEqual(actual, expected);
}

export async function authenticatePat(request: Request): Promise<PatContext | null> {
  const authorization = request.headers.get("authorization") ?? "";
  if (!authorization.startsWith("Bearer ")) return null;
  if (isDemoMode() && authorization.slice(7).trim() === DEMO_PAT) return {
    tokenId: "00000000-0000-4000-8000-000000000002",
    workspaceId: "00000000-0000-0000-0000-000000000001",
    scopes: ["context:read", "entries:write"],
    actor: { id: "00000000-0000-4000-8000-000000000003", display_name: "Demo agent", kind: "agent", platform: "test" },
  };
  const parsed = parsePat(authorization.slice(7).trim());
  if (!parsed) return null;
  const admin = createAdminClient();
  const { data: token, error } = await admin
    .from("agent_tokens")
    .select("id,workspace_id,actor_id,secret_hash,scopes,expires_at,revoked_at")
    .eq("id", parsed.tokenId)
    .maybeSingle();
  if (error || !token || token.revoked_at || new Date(token.expires_at).getTime() <= Date.now()) return null;
  if (!secretMatches(parsed.secret, token.secret_hash)) return null;
  const { data: actor, error: actorError } = await admin
    .from("actors")
    .select("id,display_name,kind,platform")
    .eq("workspace_id", token.workspace_id)
    .eq("id", token.actor_id)
    .single();
  if (actorError || !actor) return null;
  await admin.from("agent_tokens").update({ last_used_at: new Date().toISOString() }).eq("id", token.id);
  return {
    tokenId: token.id,
    workspaceId: token.workspace_id,
    scopes: token.scopes as TokenScope[],
    actor: actor as ActorRecord,
  };
}
