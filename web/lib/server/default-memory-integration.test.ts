import { beforeEach, describe, expect, it, vi } from "vitest";
import type { CommitmentReceipt } from "@reasoning/types";
import type { StoredReasoning } from "./reasoning-service";
const mock = vi.hoisted(() => ({ rows: vi.fn(), rpc: vi.fn() }));
vi.mock("./workspace", () => ({ readWorkspaceRows: mock.rows }));
vi.mock("./agent-auth", () => ({ agentClient: () => ({ rpc: mock.rpc }) }));
import { protocol, protocolAgent, protocolBundle, resultValue } from "./reasoning-test-support";
const project = { id: "project", workspace_id: "workspace", ref: "project:test", name: "Test", description: "", parent_id: null, created_at: "2026-01-01T00:00:00Z", updated_at: "2026-01-01T00:00:00Z" };
const receipt: CommitmentReceipt = { id: "server-receipt", action: "adopt", entryId: "E-commitment:v1", priorReceiptId: null, actorId: "human:owner", grantId: null, recordedAt: "2026-10-06T10:00:00-07:00", rationale: "Actual recorded approval", connectionId: null, basis: "authorized_action", sourceSpans: [] };
let stored: StoredReasoning | null;
let operations: Map<string, string>;
beforeEach(() => {
  vi.clearAllMocks(); const bundle = protocolBundle(); bundle.namespace = { datasetId: bundle.id, workspaceId: "workspace", projectId: "project" };
  stored = { bundle, head: "head-0", commitments: [receipt], authorityGrants: [] }; operations = new Map();
  mock.rows.mockImplementation(async (_client, table, workspaceId) => { expect(table).toBe("projects"); expect(workspaceId).toBe("workspace"); return [project]; });
  mock.rpc.mockImplementation(async (name, args) => {
    if (name === "reasoning_context") return { data: { mode: "reasoning_open" }, error: null };
    expect(args.target_workspace_id).toBe("workspace"); expect(args.target_project_id).toBe("project");
    if (name === "reasoning_list_datasets") return { data: { datasets: stored ? [{ id: stored.bundle.id, title: stored.bundle.title, head: stored.head }] : [], commitments: stored?.commitments ?? [], grants: [], connections: [] }, error: null };
    if (name === "reasoning_read_bundle") return { data: structuredClone(stored), error: null };
    if (name === "reasoning_append_bundle") {
      const signature = JSON.stringify([args.target_bundle, args.expected_head]);
      if (operations.has(args.operation_id)) return operations.get(args.operation_id) === signature ? { data: { head: stored!.head }, error: null } : { data: null, error: { message: "OPERATION_REUSED" } };
      if ((stored?.head ?? null) !== args.expected_head) return { data: null, error: { message: "STALE_HEAD" } };
      operations.set(args.operation_id, signature);
      stored = { bundle: structuredClone(args.target_bundle), head: "head-" + operations.size, commitments: stored?.commitments ?? [], authorityGrants: [] };
      return { data: { head: stored.head }, error: null };
    }
    throw new Error("Unexpected write: " + name);
  });
});
describe("native memory integration through service and MCP", () => {
  it("delivers authoritative guidance and a stable fingerprint using read-only RPCs", async () => {
    await protocol(async client => {
      const args = { project_ref: project.ref, task: "Assess the decision" };
      const first = resultValue<{ complete: boolean; fingerprint: string; datasets: { commitments: CommitmentReceipt[] }[] }>(await client.callTool({ name: "get_reasoning_brief", arguments: args }));
      expect(first.complete).toBe(true); expect(first.datasets[0].commitments).toEqual([receipt]);
      expect(resultValue(await client.callTool({ name: "get_reasoning_brief", arguments: { ...args, previous_fingerprint: first.fingerprint } }))).toMatchObject({ complete: true, changed: false });
      expect(mock.rpc.mock.calls.every(([name]) => ["reasoning_context", "reasoning_list_datasets", "reasoning_read_bundle"].includes(name))).toBe(true);
    });
  });
  it("imports portable authority without making it server authority, and retries without duplicate records", async () => {
    stored = null; const bundle = protocolBundle(); bundle.commitmentReceipts = [{ ...receipt, id: "portable-receipt", actorId: "fictional-human" }];
    const args = { project_ref: project.ref, bundle, expected_head: null, operation_id: "exact-import-operation" };
    await protocol(async client => {
      const first = await client.callTool({ name: "capture_reasoning", arguments: args }), repeated = await client.callTool({ name: "capture_reasoning", arguments: args });
      expect(first.isError).not.toBe(true); expect(resultValue(repeated)).toEqual(resultValue(first)); expect(operations.size).toBe(1);
      const expanded = resultValue<StoredReasoning>(await client.callTool({ name: "expand_reasoning", arguments: { project_ref: project.ref, dataset_id: bundle.id } }));
      expect(expanded.bundle.commitmentReceipts).toHaveLength(1); expect(expanded.commitments).toEqual([]);
      expect(expanded.bundle.entries).toEqual(bundle.entries); expect(expanded.bundle.sources).toEqual(bundle.sources);
      expect(expanded.bundle.namespace).toEqual({ datasetId: bundle.id, workspaceId: "workspace", projectId: "project" });
    });
  });
  it("denies a read-only agent before performing any write RPC", async () => {
    const result = await protocol(client => client.callTool({ name: "capture_reasoning", arguments: { project_ref: project.ref, bundle: protocolBundle(), expected_head: "head-0", operation_id: "denied-operation" } }), { ...protocolAgent, scopes: ["context:read"] });
    expect(result.isError).toBe(true); expect(JSON.stringify(result.content)).toContain("FORBIDDEN"); expect(mock.rpc).not.toHaveBeenCalled();
  });
  it("provides an explicit expansion inventory when governing content exceeds the byte bound", async () => {
    const entry = stored!.bundle.entries.find(item => item.id === receipt.entryId)!; entry.provenance.sourceSpans = []; entry.content = "界".repeat(80000);
    const result = await protocol(client => client.callTool({ name: "get_reasoning_brief", arguments: { project_ref: project.ref, task: "Read all guidance" } }));
    expect(result.isError).not.toBe(true); expect(resultValue(result)).toMatchObject({ complete: false, coverage_complete: false });
    expect(JSON.stringify(result.content)).toContain("Expand every listed dataset");
    expect(JSON.stringify(result.content).length).toBeLessThan(200000); expect(entry.content.length).toBe(80000);
  });
  it("reports stale writes and reused operation identities without replacing stored history", async () => {
    const original = structuredClone(stored); const bundle = protocolBundle();
    const result = await protocol(client => client.callTool({ name: "capture_reasoning", arguments: { project_ref: project.ref, bundle, expected_head: "stale", operation_id: "stale-operation" } }));
    expect(result.isError).toBe(true); expect(JSON.stringify(result.content)).toContain("CONFLICT"); expect(stored).toEqual(original);
  });
});
