import { z } from "zod";
import { requireOwnerApi } from "@/lib/server/auth";
import { apiError, privateJson, requireSameOrigin } from "@/lib/server/http";
import { getWorkspaceGraph } from "@/lib/server/data";
import { createClient } from "@/lib/supabase/server";
import { isDemoMode } from "@/lib/env";

export async function PATCH(request: Request, { params }: { params: Promise<{ id: string }> }) {
  try {
    requireSameOrigin(request);
    const owner = await requireOwnerApi();
    const { id } = await params;
    const input = z.object({ project_id: z.string().uuid().nullable(), inheritance_mode: z.enum(["local", "descendants"]), expected_updated_at: z.string() }).parse(await request.json());
    const graph = await getWorkspaceGraph(owner.workspaceId);
    const node = graph.nodes.find(n => n.id === id);
    if (!node) throw new Error("Node not found.");
    if (node.updated_at !== input.expected_updated_at) throw new Error("Node changed. Refresh before saving.");
    if (input.project_id && !graph.projects?.some(p => p.id === input.project_id)) throw new Error("Project not found.");
    if (input.inheritance_mode === "descendants" && (!input.project_id || node.type !== "Decision" || node.status !== "active")) throw new Error("Only active decisions with an owning project can apply to descendants.");
    if (isDemoMode()) return privateJson({ node: { ...node, ...input }, demo: true });
    const { data, error } = await (await createClient()).rpc("organize_node", {
      target_workspace_id: owner.workspaceId, target_node_id: id, owning_project_id: input.project_id,
      inheritance: input.inheritance_mode, expected_updated_at: input.expected_updated_at,
    });
    if (error) throw new Error(error.message);
    return privateJson({ node: data });
  } catch (error) { return apiError(error); }
}
