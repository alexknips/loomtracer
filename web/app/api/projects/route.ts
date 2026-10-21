import { randomUUID } from "node:crypto";
import { z } from "zod";
import { requireOwnerApi } from "@/lib/server/auth";
import { getWorkspaceGraph } from "@/lib/server/data";
import { apiError, privateJson, requireSameOrigin } from "@/lib/server/http";
import { createClient } from "@/lib/supabase/server";
import { isDemoMode } from "@/lib/env";
import { PROJECT_REF } from "@/lib/projects";

const projectInput = z.object({
  ref: z.string().regex(PROJECT_REF).max(160), name: z.string().trim().min(1).max(120),
  description: z.string().max(4000).default(""), parent_id: z.string().uuid().nullable().default(null),
});

export async function GET() {
  try {
    const owner = await requireOwnerApi();
    const graph = await getWorkspaceGraph(owner.workspaceId);
    return privateJson({ projects: graph.projects });
  } catch (error) { return apiError(error); }
}

export async function POST(request: Request) {
  try {
    requireSameOrigin(request);
    const owner = await requireOwnerApi();
    const input = projectInput.parse(await request.json());
    const graph = await getWorkspaceGraph(owner.workspaceId);
    if (graph.projects?.some(p => p.ref === input.ref)) throw new Error("A project with that reference already exists.");
    if (input.parent_id && !graph.projects?.some(p => p.id === input.parent_id)) throw new Error("Parent project not found.");
    const id = randomUUID();
    if (isDemoMode()) return privateJson({ project: { ...input, id }, demo: true });
    const { data, error } = await (await createClient()).rpc("save_project", {
      target_workspace_id: owner.workspaceId, target_project_id: id, project_ref: input.ref,
      project_name: input.name, project_description: input.description, parent_project_id: input.parent_id,
    });
    if (error) throw new Error(error.message);
    return privateJson({ project: data }, { status: 201 });
  } catch (error) { return apiError(error); }
}
