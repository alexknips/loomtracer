import { isDemoMode } from "@/lib/env";
import { requireOwnerApi } from "@/lib/server/auth";
import { apiError, privateJson, requireSameOrigin } from "@/lib/server/http";
import { createClient } from "@/lib/supabase/server";
import { z } from "zod";

export async function POST(request: Request, { params }: { params: Promise<{ id: string }> }) {
  try {
    requireSameOrigin(request);
    const owner = await requireOwnerApi();
    const { id } = await params;
    const body = await request.json().catch(() => ({}));
    const expectedUpdatedAt = z.iso.datetime({ offset: true }).parse(body.expected_updated_at);
    const reason = typeof body.reason === "string" ? body.reason.trim() : "";
    if (!reason) throw new Error("A rejection reason is required.");
    if (isDemoMode()) return privateJson({ node: { id, status: "rejected" } });
    const supabase = await createClient();
    const { data, error } = await supabase.rpc("reject_node", {
      target_workspace_id: owner.workspaceId,
      target_node_id: id,
      expected_updated_at: expectedUpdatedAt,
      rejection_reason: reason,
    });
    if (error) throw error;
    return privateJson({ node: data });
  } catch (error) {
    return apiError(error);
  }
}
