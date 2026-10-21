import { beforeEach, describe, expect, it, vi } from "vitest";
const mock = vi.hoisted(() => ({ projects: vi.fn(), brief: vi.fn(), datasets: vi.fn(), save: vi.fn() }));
vi.mock("./reasoning-context", () => ({ agentReasoning: () => mock }));
import { protocol, protocolAgent, protocolBundle, resultValue } from "./reasoning-test-support";
beforeEach(() => { vi.clearAllMocks(); mock.projects.mockResolvedValue([{ ref: "project:test" }]); mock.brief.mockResolvedValue({ complete: true, fingerprint: "a".repeat(64), changed: true, datasets: [] }); });
describe("native reasoning MCP protocol", () => {
  it("discovers exactly the new contract and excludes retired node mutation tools", async () => {
    await protocol(async client => {
      const { tools } = await client.listTools();
      expect(tools.map(tool => tool.name).sort()).toEqual(["capture_reasoning", "change_commitment", "expand_reasoning", "get_reasoning_brief", "get_reconstruction_packet", "list_projects", "list_reasoning_datasets", "revise_extraction", "search_reasoning"].sort());
      expect(tools.find(tool => tool.name === "get_reasoning_brief")!.annotations?.readOnlyHint).toBe(true);
      expect(tools.find(tool => tool.name === "change_commitment")!.annotations?.readOnlyHint).toBe(false);
      expect(resultValue(await client.callTool({ name: "list_projects", arguments: {} }))).toEqual({ projects: [{ ref: "project:test" }] });
      expect((await client.callTool({ name: "propose_decision", arguments: {} })).isError).toBe(true);
    });
  });
  it("round trips explicit scope and freshness without dropping the task", async () => {
    const args = { project_ref: "project:test", task: "Assess archive duration", workflow_ref: "workflow:import", target_ref: "target:archive", context_refs: ["trial:one"], previous_fingerprint: "b".repeat(64) };
    const result = await protocol(client => client.callTool({ name: "get_reasoning_brief", arguments: args }));
    expect(result.isError).not.toBe(true);
    expect(mock.brief).toHaveBeenCalledWith(args.project_ref, args.previous_fingerprint, { task: args.task, workflow_ref: args.workflow_ref, target_ref: args.target_ref, context_refs: args.context_refs });
  });
  it("rejects invalid references and missing concurrency inputs before dispatch", async () => {
    await protocol(async client => {
      expect((await client.callTool({ name: "get_reasoning_brief", arguments: { project_ref: "unscoped", task: "Review" } })).isError).toBe(true);
      expect((await client.callTool({ name: "capture_reasoning", arguments: { project_ref: "project:test", bundle: protocolBundle() } })).isError).toBe(true);
      expect(mock.brief).not.toHaveBeenCalled(); expect(mock.save).not.toHaveBeenCalled();
    });
  });
  it("reports read-only denial and preserves incomplete guidance status", async () => {
    mock.save.mockRejectedValue(new Error("FORBIDDEN"));
    const denied = await protocol(client => client.callTool({ name: "capture_reasoning", arguments: { project_ref: "project:test", bundle: protocolBundle(), expected_head: null, operation_id: "read-only-test" } }), { ...protocolAgent, scopes: ["context:read"] });
    expect(denied.isError).toBe(true); expect(JSON.stringify(denied.content)).toContain("FORBIDDEN");
    mock.brief.mockResolvedValue({ complete: false, scope_questions: [{ dimension: "workflow:" }] });
    expect(resultValue(await protocol(client => client.callTool({ name: "get_reasoning_brief", arguments: { project_ref: "project:test", task: "Review" } })))).toMatchObject({ complete: false });
  });
  it("refuses oversized results instead of truncating evidence", async () => {
    mock.projects.mockResolvedValue([{ content: "界".repeat(80000) }]);
    const result = await protocol(client => client.callTool({ name: "list_projects", arguments: {} }));
    expect(result.isError).toBe(true); expect(result.structuredContent).toBeUndefined(); expect(JSON.stringify(result.content)).toContain("coverage is incomplete");
  });
});
