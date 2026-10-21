import { requireOwnerApi } from "@/lib/server/auth";
import { apiError, PRIVATE_HEADERS } from "@/lib/server/http";
import { getWorkspaceGraph } from "@/lib/server/data";
import { projectPath } from "@/lib/projects";
import type { GraphNodeRecord, ProjectRecord } from "@/types/graph";

function markdownNode(node: GraphNodeRecord, projects: ProjectRecord[]): string {
  const lines = [`## ${node.id}: ${node.title}`, "", `- Type: ${node.type}`, `- Status: ${node.status}`];
  lines.push(`- Project: ${projectPath(projects, node.project_id)}`, `- Inheritance: ${node.inheritance_mode ?? "local"}`);
  if (node.summary) lines.push(`- Summary: ${node.summary}`);
  if (node.scope_type || node.scope_refs.length) lines.push(`- Scope: ${node.scope_type ?? "unspecified"} / ${node.scope_refs.join(", ")}`);
  if (node.reconsider_conditions.length) lines.push("- Reconsider if:", ...node.reconsider_conditions.map((item) => `  - ${item}`));
  return lines.join("\n");
}

export async function GET(request: Request) {
  try {
    const owner = await requireOwnerApi();
    const format = new URL(request.url).searchParams.get("format") ?? "jsonl";
    const { nodes, edges, projects = [] } = await getWorkspaceGraph(owner.workspaceId);
    const date = new Date().toISOString().slice(0, 10);
    if (format === "markdown") {
      const edgeLines = edges.map((edge) => `- ${edge.source_id} —${edge.edge_type}→ ${edge.target_id}${edge.summary ? `: ${edge.summary}` : ""}`);
      const projectLines = projects.map(p => `- ${p.ref}: ${projectPath(projects, p.id)} (ID: ${p.id})`);
      const body = `# Neurofilament Export\n\n# Projects\n\n${projectLines.join("\n")}\n\n${nodes.map(n => markdownNode(n, projects)).join("\n\n")}\n\n# Edges\n\n${edgeLines.join("\n")}\n`;
      return new Response(body, { headers: { ...PRIVATE_HEADERS, "Content-Type": "text/markdown; charset=utf-8", "Content-Disposition": `attachment; filename="neurofilament-${date}.md"` } });
    }
    if (format !== "jsonl") throw new Error("format must be jsonl or markdown.");
    const lines = [
      ...nodes.map((node) => JSON.stringify({ record_type: "node", ...node })),
      ...edges.map((edge) => JSON.stringify({ record_type: "edge", ...edge })),
      ...projects.map((project) => JSON.stringify({ record_type: "project", ...project })),
    ];
    return new Response(`${lines.join("\n")}\n`, { headers: { ...PRIVATE_HEADERS, "Content-Type": "application/x-ndjson; charset=utf-8", "Content-Disposition": `attachment; filename="neurofilament-${date}.jsonl"` } });
  } catch (error) {
    return apiError(error);
  }
}
