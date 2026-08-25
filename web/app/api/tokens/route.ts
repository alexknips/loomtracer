import { addDays } from "@/lib/server/token-time";
import { isDemoMode } from "@/lib/env";
import { requireOwnerApi } from "@/lib/server/auth";
import { apiError, privateJson } from "@/lib/server/http";
import { generatePat, type TokenScope } from "@/lib/server/tokens";
import { createClient } from "@/lib/supabase/server";

const EXPIRIES = new Set([30, 90, 365]);

export async function GET() {
  try {
    const owner = await requireOwnerApi();
    if (isDemoMode()) return privateJson({ tokens: [] });
    const supabase = await createClient();
    const { data, error } = await supabase.from("agent_tokens")
      .select("id,name,token_prefix,scopes,expires_at,last_used_at,revoked_at,created_at,actor_id")
      .eq("workspace_id", owner.workspaceId).order("created_at", { ascending: false });
    if (error) throw error;
    const actorIds = [...new Set((data ?? []).map((row) => row.actor_id))];
    const { data: actors, error: actorError } = actorIds.length
      ? await supabase.from("actors").select("id,display_name,kind,platform").eq("workspace_id", owner.workspaceId).in("id", actorIds)
      : { data: [], error: null };
    if (actorError) throw actorError;
    const actorMap = new Map((actors ?? []).map((actor) => [actor.id, actor]));
    return privateJson({ tokens: (data ?? []).map((row) => ({ ...row, actor: actorMap.get(row.actor_id) ?? null })) });
  } catch (error) {
    return apiError(error);
  }
}

export async function POST(request: Request) {
  try {
    const owner = await requireOwnerApi();
    const body = await request.json().catch(() => ({}));
    const name = typeof body.name === "string" ? body.name.trim() : "";
    const platform = typeof body.platform === "string" ? body.platform.trim() : "";
    const expiresDays = Number(body.expires_days);
    const canWrite = body.entries_write === true;
    if (!name) throw new Error("A token name is required.");
    if (!EXPIRIES.has(expiresDays)) throw new Error("Token expiry must be 30, 90, or 365 days.");
    const generated = generatePat();
    const scopes: TokenScope[] = canWrite ? ["context:read", "entries:write"] : ["context:read"];
    if (isDemoMode()) return privateJson({ token: generated.token, record: { id: generated.tokenId, name, scopes, expires_at: addDays(expiresDays) } }, { status: 201 });
    const supabase = await createClient();
    const { data, error } = await supabase.rpc("create_agent_token", {
      target_workspace_id: owner.workspaceId,
      target_token_id: generated.tokenId,
      token_name: name,
      actor_name: name,
      actor_platform: platform,
      target_token_prefix: generated.prefix,
      target_secret_hash: generated.secretHash,
      target_scopes: scopes,
      target_expires_at: addDays(expiresDays),
    });
    if (error) throw error;
    return privateJson({ token: generated.token, record: data }, { status: 201 });
  } catch (error) {
    return apiError(error);
  }
}
