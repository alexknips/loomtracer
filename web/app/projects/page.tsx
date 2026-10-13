import { AppShell } from "@/components/app-shell";
import { ProjectManager } from "@/components/project-manager";
import { requireOwner } from "@/lib/server/auth";
import { getWorkspaceGraph } from "@/lib/server/data";

export const dynamic = "force-dynamic";
export default async function ProjectsPage() {
  const owner = await requireOwner();
  const graph = await getWorkspaceGraph(owner.workspaceId);
  return <AppShell active="Projects"><ProjectManager projects={graph.projects ?? []} nodes={graph.nodes} edges={graph.edges} /></AppShell>;
}
