import { z } from "zod";
import { requireOwnerApi } from "@/lib/server/auth";
import { getWorkspaceGraph } from "@/lib/server/data";
import { apiError, privateJson, requireSameOrigin } from "@/lib/server/http";
import { createClient } from "@/lib/supabase/server";
import { isDemoMode } from "@/lib/env";
import { moveImpact } from "@/lib/projects";

const inputSchema = z.object({ name: z.string().trim().min(1).max(120), description: z.string().max(4000),
  parent_id: z.string().uuid().nullable(), expected_updated_at: z.string(), preview: z.boolean().default(false) });

export async function PATCH(request: Request, { params }: { params: Promise<{ id: string }> }) {
  try {
    requireSameOrigin(request);
    const owner = await requireOwnerApi();
    const { id } = await params;
    const input = inputSchema.parse(await request.json());
    const graph = await getWorkspaceGraph(owner.workspaceId);
    const projects = graph.projects ?? [];
    const current = projects.find(p => p.id === id);
    if (!current) throw new Error("Project not found.");
    if (current.updated_at !== input.expected_updated_at) throw new Error("Project changed. Refresh and review again.");
    const impact = moveImpact(projects, graph.nodes, id, input.parent_id);
    if (input.preview) return privateJson({ impact });
    if (isDemoMode()) return privateJson({ project: { ...current, ...input }, demo: true });
    const { data, error } = await (await createClient()).rpc("save_project", {
      target_workspace_id: owner.workspaceId, target_project_id: id, project_ref: current.ref,
      project_name: input.name, project_description: input.description, parent_project_id: input.parent_id,
      expected_updated_at: input.expected_updated_at,
    });
    if (error) throw new Error(error.message);
    return privateJson({ project: data });
  } catch (error) { return apiError(error); }
}
