import { isDemoMode } from "@/lib/env";
import { requireOwnerApi } from "@/lib/server/auth";
import { apiError, privateJson } from "@/lib/server/http";
import { createClient } from "@/lib/supabase/server";

export async function POST(request: Request, { params }: { params: Promise<{ id: string }> }) {
  try {
    const owner = await requireOwnerApi();
    const { id } = await params;
    const body = await request.json().catch(() => ({}));
    const reason = typeof body.reason === "string" ? body.reason.trim() : "";
    if (!reason) throw new Error("A rejection reason is required.");
    if (isDemoMode()) return privateJson({ node: { id, status: "rejected" } });
    const supabase = await createClient();
    const { data, error } = await supabase.rpc("reject_node", {
      target_workspace_id: owner.workspaceId,
      target_node_id: id,
      rejection_reason: reason,
    });
    if (error) throw error;
    return privateJson({ node: data });
  } catch (error) {
    return apiError(error);
  }
}
