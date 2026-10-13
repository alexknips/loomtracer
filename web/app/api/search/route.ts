import { NextRequest } from "next/server";
import { requireOwnerApi } from "@/lib/server/auth";
import { apiError, privateJson } from "@/lib/server/http";
import { getWorkspaceGraph } from "@/lib/server/data";
import { selectProjectNodes, projectPath } from "@/lib/projects";
import { rankHostedNodes } from "@/lib/server/hosted-graph";

export async function GET(request: NextRequest) {
  try {
    const owner = await requireOwnerApi();
    const query = (request.nextUrl.searchParams.get("q") ?? "").trim();
    if (query.length < 2) return privateJson({ results: [] });
    const graph = await getWorkspaceGraph(owner.workspaceId);
    const projects = graph.projects ?? [];
    const nodes = selectProjectNodes(graph.nodes, projects, request.nextUrl.searchParams.get("project") ?? "all", true);
    return privateJson({ results: rankHostedNodes(nodes, query, 20).map(({ node }) => ({
      id: node.id, type: node.type, status: node.status, title: node.title, summary: node.summary,
      project_id: node.project_id, project_path: projectPath(projects, node.project_id),
      inherited: Boolean(node.inherited_from_project_id), updated_at: node.updated_at,
    })) });
  } catch (error) { return apiError(error); }
}
