import { isDemoMode } from "@/lib/env";
import { requireOwnerApi } from "@/lib/server/auth";
import { apiError, privateJson } from "@/lib/server/http";
import { createClient } from "@/lib/supabase/server";

export async function DELETE(_request: Request, { params }: { params: Promise<{ id: string }> }) {
  try {
    const owner = await requireOwnerApi();
    const { id } = await params;
    if (isDemoMode()) return privateJson({ token: { id, revoked_at: new Date().toISOString() } });
    const supabase = await createClient();
    const { data, error } = await supabase.rpc("revoke_agent_token", {
      target_workspace_id: owner.workspaceId,
      target_token_id: id,
    });
    if (error) throw error;
    return privateJson({ token: data });
  } catch (error) {
    return apiError(error);
  }
}
