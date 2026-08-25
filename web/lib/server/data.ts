import { demoGraph, demoReview } from "@/lib/demo";
import { createClient } from "@/lib/supabase/server";
import type { ActorRecord, AuditEventRecord, GraphNodeRecord, GraphPayload, ReviewRecord } from "@/types/graph";

export async function getGraphPayload(
  workspaceId: string,
  rootId?: string | null,
  options: { depth?: number; maxNodes?: number; maxEdges?: number } = {},
): Promise<GraphPayload> {
  if (process.env.NODE_ENV !== "production" && process.env.NEUROFILAMENT_DEMO_MODE === "true") return demoGraph;
  const supabase = await createClient();
  let resolvedRoot = rootId;
  if (!resolvedRoot) {
    const { data, error } = await supabase
      .from("nodes")
      .select("id")
      .eq("workspace_id", workspaceId)
      .eq("type", "Decision")
      .order("updated_at", { ascending: false })
      .limit(1)
      .maybeSingle();
    if (error) throw error;
    if (!data) {
      const fallback = await supabase.from("nodes").select("id").eq("workspace_id", workspaceId)
        .order("updated_at", { ascending: false }).limit(1).maybeSingle();
      if (fallback.error) throw fallback.error;
      resolvedRoot = fallback.data?.id ?? null;
    } else resolvedRoot = data.id;
  }
  if (!resolvedRoot) return { root_id: null, nodes: [], edges: [], audits: {}, truncated: false };

  const { data, error } = await supabase.rpc("get_graph_subgraph", {
    target_workspace_id: workspaceId,
    root_node_id: resolvedRoot,
    max_depth: Math.min(Math.max(options.depth ?? 2, 0), 5),
    max_nodes: Math.min(Math.max(options.maxNodes ?? 200, 1), 200),
    max_edges: Math.min(Math.max(options.maxEdges ?? 400, 0), 400),
  });
  if (error) throw error;
  const payload = data as Omit<GraphPayload, "audits">;
  const nodes = (payload.nodes ?? []) as GraphNodeRecord[];
  const actorIds = [...new Set(nodes.map((node) => node.created_by_actor_id).filter(Boolean))] as string[];
  const nodeIds = nodes.map((node) => node.id);
  const [actorsResult, auditResult] = await Promise.all([
    actorIds.length
      ? supabase.from("actors").select("id,display_name,kind,platform").eq("workspace_id", workspaceId).in("id", actorIds)
      : Promise.resolve({ data: [], error: null }),
    nodeIds.length
      ? supabase.from("audit_events").select("id,action,actor_label,reason,object_id,created_at")
        .eq("workspace_id", workspaceId).in("object_id", nodeIds).order("created_at", { ascending: false })
      : Promise.resolve({ data: [], error: null }),
  ]);
  if (actorsResult.error) throw actorsResult.error;
  if (auditResult.error) throw auditResult.error;
  const actorMap = new Map((actorsResult.data as ActorRecord[]).map((actor) => [actor.id, actor]));
  const audits: Record<string, AuditEventRecord[]> = {};
  for (const event of auditResult.data ?? []) {
    const objectId = String(event.object_id);
    (audits[objectId] ??= []).push(event as AuditEventRecord);
  }
  return {
    ...payload,
    root_id: resolvedRoot,
    nodes: nodes.map((node) => ({ ...node, actor: node.created_by_actor_id ? actorMap.get(node.created_by_actor_id) ?? null : null })),
    audits,
  };
}

export async function getReviewRecords(workspaceId: string): Promise<ReviewRecord[]> {
  if (process.env.NODE_ENV !== "production" && process.env.NEUROFILAMENT_DEMO_MODE === "true") return demoReview;
  const supabase = await createClient();
  const { data, error } = await supabase.from("nodes").select("*")
    .eq("workspace_id", workspaceId)
    .in("type", ["Decision", "Assumption"])
    .in("status", ["candidate", "proposed"])
    .order("created_at", { ascending: false });
  if (error) throw error;
  const rows = (data ?? []) as GraphNodeRecord[];
  const actorIds = [...new Set(rows.map((row) => row.created_by_actor_id).filter(Boolean))] as string[];
  const { data: actors, error: actorError } = actorIds.length
    ? await supabase.from("actors").select("id,display_name,kind,platform").eq("workspace_id", workspaceId).in("id", actorIds)
    : { data: [], error: null };
  if (actorError) throw actorError;
  const actorMap = new Map((actors as ActorRecord[]).map((actor) => [actor.id, actor]));
  const supersedesIds = [...new Set(rows.map((row) => row.metadata.proposed_supersedes).filter((value): value is string => typeof value === "string"))];
  const { data: superseded, error: supersededError } = supersedesIds.length
    ? await supabase.from("nodes").select("*").eq("workspace_id", workspaceId).in("id", supersedesIds)
    : { data: [], error: null };
  if (supersededError) throw supersededError;
  const supersededMap = new Map(((superseded ?? []) as GraphNodeRecord[]).map((node) => [node.id, node]));
  return rows.map((row) => ({
    ...row,
    actor: row.created_by_actor_id ? actorMap.get(row.created_by_actor_id) ?? null : null,
    proposed_supersedes: typeof row.metadata.proposed_supersedes === "string"
      ? supersededMap.get(row.metadata.proposed_supersedes) ?? null
      : null,
  }));
}
