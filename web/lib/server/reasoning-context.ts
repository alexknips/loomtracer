import { requireOwnerApi } from "./auth";
import { createClient } from "@/lib/supabase/server";
import { agentClient, type AgentContext } from "./agent-auth";
import { ReasoningService } from "./reasoning-service";
export async function humanReasoning() {
  const owner = await requireOwnerApi();
  return new ReasoningService(await createClient(), { workspaceId: owner.workspaceId,
    actor: { id: `human:${owner.user.id}`, name: "Workspace member", kind: "human" },
    // Preserve admitted human teammates' existing capture and review rights.
    // Membership is resolved server-side; a login alone is insufficient.
    isOwner: owner.membershipRole === "owner",
    canWrite: owner.membershipRole === "owner" || owner.membershipRole === "reviewer", agent: false });
}
export function agentReasoning(agent: AgentContext) {
  return new ReasoningService(agentClient(agent.accessToken), { workspaceId: agent.workspaceId,
    actor: { id: `agent:${agent.actor.id}`, name: agent.actor.display_name, kind: "agent" },
    isOwner: false, canWrite: agent.scopes.includes("entries:write"), agent: true });
}
