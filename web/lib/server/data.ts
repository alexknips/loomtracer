import { demoGraph } from "@/lib/demo";
import { createClient } from "@/lib/supabase/server";
import { isDemoMode } from "@/lib/env";
import { projectGraph } from "@/lib/projects";
import { readWorkspaceGraph } from "@/lib/server/workspace";
import type { GraphPayload, ReviewRecord } from "@/types/graph";

export async function getWorkspaceGraph(workspaceId: string): Promise<GraphPayload> {
  return isDemoMode() ? structuredClone(demoGraph) : readWorkspaceGraph(await createClient(), workspaceId);
}

export async function getGraphPayload(
  workspaceId: string, rootId?: string | null,
  options: { view?: "all" | "neighborhood"; depth?: number; maxNodes?: number; maxEdges?: number; project?: string; includeInherited?: boolean } = {},
): Promise<GraphPayload> {
  const payload = await getWorkspaceGraph(workspaceId);
  return projectGraph(payload, options.project ?? "all", {
    ...options, root: rootId, view: options.view ?? (options.depth === undefined ? "all" : "neighborhood"),
  });
}

export async function getReviewRecords(workspaceId: string, payload?: GraphPayload): Promise<ReviewRecord[]> {
  const graph = payload ?? await getWorkspaceGraph(workspaceId);
  const byId = new Map(graph.nodes.map(node => [node.id, node]));
  return graph.nodes.filter(node => (node.type === "Decision" || node.type === "Assumption")
    && (node.status === "candidate" || node.status === "proposed"))
    .sort((a, b) => b.created_at.localeCompare(a.created_at))
    .map(node => ({ ...node, proposed_supersedes: typeof node.metadata.proposed_supersedes === "string"
      ? byId.get(node.metadata.proposed_supersedes) ?? null : null }));
}
