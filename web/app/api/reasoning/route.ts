import { z } from "zod";
import { apiError, privateJson } from "@/lib/http";
import { buildProjectBundle } from "@/lib/hivemind/bundle";
import { hivemind } from "@/lib/hivemind/client";
import { listProjects, projectPath } from "@/lib/hivemind/projects";

export const dynamic = "force-dynamic";

const projectRef = z.string().min(1).max(200);

/**
 * Read-only. Without `project_ref`: the projects of the tenant this workspace's key reads.
 * With it: that project's history as a reasoning bundle, and the ledger position it was read at.
 */
export async function GET(request: Request) {
  try {
    const client = hivemind();
    const [graph, who] = await Promise.all([client.graph(), client.whoami()]);
    const projects = listProjects(graph);
    const wanted = new URL(request.url).searchParams.get("project_ref");
    if (wanted === null) {
      return privateJson({ projects: projects.map(project => ({ ref: project.ref, name: projectPath(projects, project.ref), decisions: project.counts.total })) });
    }
    const ref = projectRef.safeParse(wanted);
    if (!ref.success) return privateJson({ error: "Name a project." }, { status: 400 });
    const built = buildProjectBundle(graph, ref.data, who.tenant_id, new Date().toISOString());
    if (!built) return privateJson({ error: "That project is not in the ledger.", code: "not_found" }, { status: 404 });
    return privateJson({ bundle: built.bundle, head: built.head });
  } catch (error) {
    return apiError(error);
  }
}
