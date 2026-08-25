import { isDemoMode } from "@/lib/env";
import { demoGraph } from "@/lib/demo";
import { requireOwnerApi } from "@/lib/server/auth";
import { apiError, PRIVATE_HEADERS } from "@/lib/server/http";
import { createClient } from "@/lib/supabase/server";
import type { GraphEdgeRecord, GraphNodeRecord } from "@/types/graph";

function markdownNode(node: GraphNodeRecord): string {
  const lines = [`## ${node.id}: ${node.title}`, "", `- Type: ${node.type}`, `- Status: ${node.status}`];
  if (node.summary) lines.push(`- Summary: ${node.summary}`);
  if (node.scope_type || node.scope_refs.length) lines.push(`- Scope: ${node.scope_type ?? "unspecified"} / ${node.scope_refs.join(", ")}`);
  if (node.reconsider_conditions.length) lines.push("- Reconsider if:", ...node.reconsider_conditions.map((item) => `  - ${item}`));
  return lines.join("\n");
}

export async function GET(request: Request) {
  try {
    const owner = await requireOwnerApi();
    const format = new URL(request.url).searchParams.get("format") ?? "jsonl";
    let nodes: GraphNodeRecord[];
    let edges: GraphEdgeRecord[];
    if (isDemoMode()) ({ nodes, edges } = demoGraph);
    else {
      const supabase = await createClient();
      const [nodeResult, edgeResult] = await Promise.all([
        supabase.from("nodes").select("*").eq("workspace_id", owner.workspaceId).order("created_at"),
        supabase.from("edges").select("*").eq("workspace_id", owner.workspaceId).order("created_at"),
      ]);
      if (nodeResult.error) throw nodeResult.error;
      if (edgeResult.error) throw edgeResult.error;
      nodes = (nodeResult.data ?? []) as GraphNodeRecord[];
      edges = (edgeResult.data ?? []) as GraphEdgeRecord[];
    }
    const date = new Date().toISOString().slice(0, 10);
    if (format === "markdown") {
      const edgeLines = edges.map((edge) => `- ${edge.source_id} —${edge.edge_type}→ ${edge.target_id}${edge.summary ? `: ${edge.summary}` : ""}`);
      const body = `# Neurofilament Export\n\n${nodes.map(markdownNode).join("\n\n")}\n\n# Edges\n\n${edgeLines.join("\n")}\n`;
      return new Response(body, { headers: { ...PRIVATE_HEADERS, "Content-Type": "text/markdown; charset=utf-8", "Content-Disposition": `attachment; filename="neurofilament-${date}.md"` } });
    }
    if (format !== "jsonl") throw new Error("format must be jsonl or markdown.");
    const lines = [
      ...nodes.map((node) => JSON.stringify({ record_type: "node", ...node })),
      ...edges.map((edge) => JSON.stringify({ record_type: "edge", ...edge })),
    ];
    return new Response(`${lines.join("\n")}\n`, { headers: { ...PRIVATE_HEADERS, "Content-Type": "application/x-ndjson; charset=utf-8", "Content-Disposition": `attachment; filename="neurofilament-${date}.jsonl"` } });
  } catch (error) {
    return apiError(error);
  }
}
