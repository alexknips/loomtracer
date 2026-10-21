import { afterEach, describe, expect, it, vi } from "vitest";
import type { AgentContext } from "./agent-auth";
const rpc = vi.hoisted(() => vi.fn());
vi.mock("./agent-auth", () => ({ agentClient: () => ({ rpc }) }));
import { recordCompleteBrief } from "./brief-verification";

const agent = { accessToken: "test-only" } as AgentContext;
afterEach(() => { vi.unstubAllEnvs(); vi.clearAllMocks(); });
describe("complete briefing telemetry", () => {
  it("records only a complete hosted brief through the caller's database session", async () => {
    rpc.mockResolvedValue({ error: null });
    expect(await recordCompleteBrief(agent, "project:demo", { complete: false, fingerprint: "abc" })).toBe(false);
    expect(rpc).not.toHaveBeenCalled();
    expect(await recordCompleteBrief(agent, "project:demo", { complete: true, fingerprint: "abc" })).toBe(true);
    expect(rpc).toHaveBeenCalledWith("record_agent_brief_verified", { target_project_ref: "project:demo", brief_fingerprint: "abc" });
  });
  it("does not mark fixtures or failed database writes as verified", async () => {
    vi.stubEnv("NEUROFILAMENT_DEMO_MODE", "true");
    expect(await recordCompleteBrief(agent, "project:demo", { complete: true, fingerprint: "abc" })).toBe(false);
    expect(rpc).not.toHaveBeenCalled();
    vi.stubEnv("NEUROFILAMENT_DEMO_MODE", "false");
    rpc.mockRejectedValue(new Error("offline"));
    expect(await recordCompleteBrief(agent, "project:demo", { complete: true, fingerprint: "abc" })).toBe(false);
  });
});
