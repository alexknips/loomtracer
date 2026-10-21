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
    const conditions = body.reconsider_conditions;
    if (conditions !== undefined && (!Array.isArray(conditions) || !conditions.every((item) => typeof item === "string"))) {
      throw new Error("reconsider_conditions must be a list of strings.");
    }
    if (isDemoMode()) return privateJson({ node: { id, status: "active" } });
    const supabase = await createClient();
    const { data, error } = await supabase.rpc("ratify_node", {
      target_workspace_id: owner.workspaceId,
      target_node_id: id,
      expected_updated_at: expectedUpdatedAt,
      supplied_reconsider_conditions: conditions?.map((item: string) => item.trim()).filter(Boolean) ?? null,
    });
    if (error) throw error;
    return privateJson({ node: data });
  } catch (error) {
    return apiError(error);
  }
}
