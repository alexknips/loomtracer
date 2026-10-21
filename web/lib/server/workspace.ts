import type { SupabaseClient } from "@supabase/supabase-js";
import type { ActorRecord, AuditEventRecord, GraphEdgeRecord, GraphNodeRecord, GraphPayload, ProjectRecord } from "@/types/graph";

// Page explicitly: PostgREST can cap responses below a requested limit.
export async function readWorkspaceRows<T>(client: SupabaseClient, table: string, workspaceId: string, filter?: { column: string; values: string[] }): Promise<T[]> {
  const rows: T[] = [];
  let expected: number | null = null;
  while (rows.length < 20000) {
    let query = client.from(table).select("*", { count: "exact" }).eq("workspace_id", workspaceId);
    if (filter) query = query.in(filter.column, filter.values);
    const { data, count, error } = await query.order("id").range(rows.length, rows.length + 249);
    if (error) throw new Error(`${table}: ${error.message}`);
    if (count === null || (expected !== null && expected !== count)) throw new Error(`${table} changed during retrieval. Please retry.`);
    expected = count;
    rows.push(...(data ?? []) as T[]);
    if (rows.length === count) {
      if (new Set(rows.map(row => (row as { id: string }).id)).size !== rows.length) break;
      return rows;
    }
    if (!data?.length || rows.length > count) break;
  }
  throw new Error(`${table} could not be read completely. Narrow the workspace or retry; results are incomplete.`);
}

export async function readWorkspaceGraph(client: SupabaseClient, workspaceId: string, includeAudits = true): Promise<GraphPayload> {
  const [nodes, edges, projects, actors, events] = await Promise.all([
    readWorkspaceRows<GraphNodeRecord>(client, "nodes", workspaceId),
    readWorkspaceRows<GraphEdgeRecord>(client, "edges", workspaceId),
    readWorkspaceRows<ProjectRecord>(client, "projects", workspaceId),
    readWorkspaceRows<ActorRecord>(client, "actors", workspaceId),
    includeAudits ? readWorkspaceRows<AuditEventRecord>(client, "audit_events", workspaceId) : Promise.resolve([]),
  ]);
  const actorMap = new Map(actors.map(a => [a.id, a]));
  const audits: Record<string, AuditEventRecord[]> = {};
  for (const event of events) if (event.object_id) (audits[event.object_id] ??= []).push(event);
  for (const list of Object.values(audits)) list.sort((a, b) => b.created_at.localeCompare(a.created_at));
  return { nodes: nodes.map(n => ({ ...n, actor: n.created_by_actor_id ? actorMap.get(n.created_by_actor_id) ?? null : null })),
    edges, projects, audits, root_id: null, truncated: false,
    warnings: nodes.some(n => !n.project_id) ? ["Some records are unassigned. Review them before assuming project guidance is complete."] : [],
  };
}
