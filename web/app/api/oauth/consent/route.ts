import { z } from "zod";
import { requireOwnerApi } from "@/lib/server/auth";
import { createClient } from "@/lib/supabase/server";
import { apiError, privateJson, requireSameOrigin } from "@/lib/server/http";
import { OAUTH_AUTHORIZATION_ID } from "@/lib/server/auth-policy";
export async function POST(request: Request) {
  try {
    requireSameOrigin(request); const owner = await requireOwnerApi();
    const { authorizationId, approve, allowWrites, expectedWorkspaceId, expectedUserId } = z.object({ authorizationId: z.string().regex(OAUTH_AUTHORIZATION_ID), approve: z.boolean(), allowWrites: z.boolean(), expectedWorkspaceId: z.uuid(), expectedUserId: z.uuid() }).parse(await request.json());
    if (expectedWorkspaceId !== owner.workspaceId || expectedUserId !== owner.user.id) throw new Error("FORBIDDEN");
    const supabase = await createClient();
    const { data: details, error } = await supabase.auth.oauth.getAuthorizationDetails(authorizationId);
    if (error || !details || !("authorization_id" in details) || details.user.id !== owner.user.id) throw new Error("FORBIDDEN");
    if (!approve) {
      const { data, error } = await supabase.auth.oauth.denyAuthorization(authorizationId, { skipBrowserRedirect: true });
      if (error) throw error;
      return privateJson(data);
    }
    const { data: connection, error: grantError } = await supabase.rpc("approve_agent_connection", {
      oauth_client_id: details.client.id, display_name: details.client.name || "Agent", allow_writes: allowWrites,
    });
    if (grantError) throw grantError;
    const { data, error: approvalError } = await supabase.auth.oauth.approveAuthorization(authorizationId, { skipBrowserRedirect: true });
    if (approvalError) {
      await supabase.rpc("disconnect_agent", { connection_id: connection.id });
      throw approvalError;
    }
    return privateJson(data);
  } catch (error) { return apiError(error); }
}
