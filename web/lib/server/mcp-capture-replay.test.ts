import { beforeEach, describe, expect, it, vi } from "vitest";
const mock = vi.hoisted(() => ({ save: vi.fn(), appendDelta: vi.fn(), commit: vi.fn() }));
vi.mock("./reasoning-context", () => ({ agentReasoning: () => mock }));
import { protocol, protocolBundle, resultValue } from "./reasoning-test-support";
beforeEach(() => vi.clearAllMocks());
describe("native operation replay", () => {
  const delta = { entries: [], run: { id: "run-2", recordedBy: "agent:test", createdAt: "2026-10-12T00:00:00Z", notes: "Incremental capture" } };
  it.each(["capture_reasoning", "revise_extraction"])("accepts small append requests for %s without a full transcript", async name => {
    mock.appendDelta.mockResolvedValue({ dataset_id: "large-existing", head: "saved-head", authorityChanged: false });
    const args = { project_ref: "project:test", dataset_id: "large-existing", delta, expected_head: "head-before", operation_id: "delta-operation" };
    await protocol(async client => {
      const first = await client.callTool({ name, arguments: args });
      const retry = await client.callTool({ name, arguments: args });
      expect(first.isError).not.toBe(true); expect(resultValue(retry)).toEqual(resultValue(first));
      expect(mock.appendDelta).toHaveBeenCalledWith("project:test", "large-existing", delta, "head-before", "delta-operation");
      expect(mock.save).not.toHaveBeenCalled();
    });
  });
  it("rejects ambiguous full-plus-delta requests and missing delta heads", async () => {
    await protocol(async client => {
      for (const extra of [{ bundle: protocolBundle(), expected_head: "head" }, { expected_head: null }]) {
        const result = await client.callTool({ name: "capture_reasoning", arguments: { project_ref: "project:test", dataset_id: "existing", delta, operation_id: "delta-operation", ...extra } });
        expect(result.isError).toBe(true);
      }
    });
    expect(mock.appendDelta).not.toHaveBeenCalled(); expect(mock.save).not.toHaveBeenCalled();
  });
  it.each(["capture_reasoning", "revise_extraction"])("retains stable identity and concurrency inputs across %s retries", async name => {
    const bundle = protocolBundle(); mock.save.mockResolvedValue({ bundle, head: "same-head", commitments: [] });
    const args = { project_ref: "project:test", bundle, expected_head: null, operation_id: "same-operation" };
    await protocol(async client => {
      const first = await client.callTool({ name, arguments: args }), second = await client.callTool({ name, arguments: args });
      expect(first.isError).not.toBe(true); expect(resultValue(second)).toEqual(resultValue(first));
      expect(mock.save.mock.calls[0]).toEqual(mock.save.mock.calls[1]);
      expect(resultValue(second)).toMatchObject({ authorityChanged: false, head: "same-head" });
    });
  });
  it("returns the actual server commitment receipt on retry without manufacturing an action", async () => {
    const receipt = { id: "receipt-original", action: "revise", priorReceiptId: "prior", actorId: "agent:delegated" };
    mock.commit.mockResolvedValue({ result: { receipt, head: "new-head" }, head: "new-head", commitments: [receipt] });
    const args = { project_ref: "project:test", dataset_id: "dataset", action: "revise", entry_id: "new-contribution", prior_receipt_id: "prior", rationale: "Changed evidence", expected_head: "old-head", operation_id: "same-operation" };
    await protocol(async client => {
      const first = await client.callTool({ name: "change_commitment", arguments: args }), second = await client.callTool({ name: "change_commitment", arguments: args });
      expect(resultValue(second)).toEqual(resultValue(first)); expect(resultValue(second)).toMatchObject({ commitments: [receipt] });
      expect(mock.commit.mock.calls[0]).toEqual(mock.commit.mock.calls[1]);
    });
  });
  it.each(["STALE_HEAD", "OPERATION_REUSED", "IMMUTABLE record", "operation id reused with a different request", "immutable record changed: entries/E1", "history cannot be removed from a dataset"])("reports %s without claiming successful capture", async message => {
    mock.save.mockRejectedValue(new Error(message));
    const result = await protocol(client => client.callTool({ name: "capture_reasoning", arguments: { project_ref: "project:test", bundle: protocolBundle(), expected_head: "old", operation_id: "same-operation" } }));
    expect(result.isError).toBe(true); expect(JSON.stringify(result.content)).toContain("CONFLICT"); expect(result.structuredContent).toBeUndefined();
  });
});
