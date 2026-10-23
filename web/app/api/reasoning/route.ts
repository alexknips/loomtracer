import { z } from "zod";
import { apiError, privateJson } from "@/lib/http";
import { buildProjectBundle } from "@/lib/loomtracer/bundle";
import { loomtracer } from "@/lib/loomtracer/client";
import { projectPath, summarizeProjects } from "@/lib/loomtracer/projects";

export const dynamic = "force-dynamic";

const projectRef = z.string().min(1).max(200);
/** How many of a project's newest decisions to draw. Loomtracer's own ceiling is 1000. */
const decisionLimit = z.coerce.number().int().min(1).max(1000);

/**
 * Read-only. Without `project_ref`: the projects of the tenant this workspace's key reads.
 * With it: that project's newest decisions (`limit`, default Loomtracer's page) as a reasoning
 * bundle, the ledger position it was read at, and how many of the project's decisions are shown.
 */
export async function GET(request: Request) {
  try {
    const client = loomtracer();
    const params = new URL(request.url).searchParams;
    const wanted = params.get("project_ref");
    if (wanted === null) {
      const { items } = await client.projects();
      const projects = summarizeProjects(items);
      return privateJson({ projects: projects.map(project => ({ ref: project.ref, name: projectPath(projects, project.ref), decisions: project.counts.total })) });
    }
    const ref = projectRef.safeParse(wanted);
    if (!ref.success) return privateJson({ error: "Name a project." }, { status: 400 });
    const asked = params.get("limit");
    const limit = asked === null ? { success: true as const, data: undefined } : decisionLimit.safeParse(asked);
    if (!limit.success) return privateJson({ error: "limit must be a whole number from 1 to 1000." }, { status: 400 });
    const [graph, listed, who] = await Promise.all([client.graph({ project: ref.data, limit: limit.data }), client.projects(), client.whoami()]);
    const projects = summarizeProjects(listed.items);
    const project = projects.find(item => item.ref === ref.data);
    if (!project) return privateJson({ error: "That project is not in the ledger.", code: "not_found" }, { status: 404 });
    const built = buildProjectBundle(graph, project, who.tenant_id, new Date().toISOString());
    // The first page holds the project's newest `limit` decisions. A decision of the same project can
    // also appear as the far end of an edge; it is named there, not one of the page's, so it is not counted.
    const total = graph.total_matches ?? 0;
    const shown = Math.min(graph.limit ?? total, total);
    return privateJson({ bundle: built.bundle, head: built.head, decisions: { shown, total, truncated: graph.truncated ?? false } });
  } catch (error) {
    return apiError(error);
  }
}
