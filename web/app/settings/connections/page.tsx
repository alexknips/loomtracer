import { AppShell } from "@/components/app-shell";
import { ConnectionManager, type AgentConnection } from "@/components/connection-manager";
import { requireOwner } from "@/lib/server/auth";
import { createClient } from "@/lib/supabase/server";
import { mcpResource } from "@/lib/server/agent-auth";
import { readWorkspaceRows } from "@/lib/server/workspace";
import { demoGraph } from "@/lib/demo";
import type { ProjectRecord } from "@/types/graph";

export const dynamic = "force-dynamic";
export default async function ConnectionsPage() {
  const member = await requireOwner();
  const database = member.demo ? null : await createClient();
  const [connections, projects] = database ? await Promise.all([
    readWorkspaceRows<AgentConnection>(database, "agent_connections", member.workspaceId, { column: "user_id", values: [member.user.id] }),
    readWorkspaceRows<ProjectRecord>(database, "projects", member.workspaceId),
  ]) : [[], demoGraph.projects ?? []];
  return <AppShell active="Agent connections"><ConnectionManager
    connections={connections.sort((a, b) => b.granted_at.localeCompare(a.granted_at))}
    projects={projects.map(({ ref, name }) => ({ ref, name })).sort((a, b) => a.name.localeCompare(b.name))}
    endpoint={member.demo ? "http://localhost:3000/api/mcp" : mcpResource()}
    workspaceName={member.workspaceName} membershipRole={member.membershipRole}
  /></AppShell>;
}
