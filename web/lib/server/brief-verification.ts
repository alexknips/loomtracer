import { isDemoMode } from "@/lib/env";
import { agentClient, type AgentContext } from "./agent-auth";

export async function recordCompleteBrief(agent: AgentContext, projectRef: string, brief: { complete: boolean; fingerprint: string }) {
  if (!brief.complete || isDemoMode()) return false;
  try {
    const { error } = await agentClient(agent.accessToken).rpc("record_agent_brief_verified", {
      target_project_ref: projectRef,
      brief_fingerprint: brief.fingerprint,
    });
    return !error;
  } catch {
    // Setup telemetry must never turn successfully retrieved guidance into a failure.
    return false;
  }
}
