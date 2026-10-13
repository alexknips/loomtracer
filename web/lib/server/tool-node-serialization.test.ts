import { beforeEach, describe, expect, it, vi } from "vitest";
import { buildEvaluationPacket, entryText } from "@reasoning/domain";
const mock = vi.hoisted(() => ({ read: vi.fn(), packet: vi.fn(), search: vi.fn() }));
vi.mock("./reasoning-context", () => ({ agentReasoning: () => mock }));
import { protocol, protocolBundle, resultValue } from "./reasoning-test-support";
let bundle: ReturnType<typeof protocolBundle>;
beforeEach(() => { vi.clearAllMocks(); bundle = protocolBundle(); mock.read.mockResolvedValue({ bundle, head: "head", commitments: [], authorityGrants: [] }); });
describe("reasoning record serialization", () => {
  it("returns a paginated inventory for a large dataset without shipping attachment bytes", async () => {
    bundle.attachments.push({ id: "large-image", mimeType: "image/png", dataUrl: "data:image/png;base64," + "A".repeat(220000), sha256: "a".repeat(64) });
    await protocol(async client => {
      const first = resultValue(await client.callTool({ name: "expand_reasoning", arguments: { project_ref: "project:test", dataset_id: bundle.id, limit: 2 } }));
      expect(first).toMatchObject({ coverage_complete: false, head: "head", next_offset: 2, nextSourceOrder: 5 });
      expect(first.records).toHaveLength(2); expect(first.contexts).toEqual(bundle.contexts);
      expect(JSON.stringify(first)).not.toContain("base64");
      const next = resultValue(await client.callTool({ name: "expand_reasoning", arguments: { project_ref: "project:test", dataset_id: bundle.id, offset: 2, limit: 2 } }));
      expect(next.records).not.toEqual(first.records); expect(next.next_offset).toBe(4);
    });
  });
  it("preserves exact text, attribution, version identity, inverse links and source context on expansion", async () => {
    const entry = bundle.entries.find(item => item.id === "E-recommendation:v1")!;
    const result = await protocol(client => client.callTool({ name: "expand_reasoning", arguments: { project_ref: "project:test", dataset_id: bundle.id, record_id: entry.id } }));
    const expanded = resultValue<{ entry: unknown; sources: typeof bundle.sources; relatedConnections: typeof bundle.connections }>(result);
    expect(expanded.entry).toEqual({ ...entry, content: entryText(bundle, entry) });
    expect(expanded.sources).toEqual(bundle.sources.filter(source => entry.provenance.sourceSpans.some(span => span.messageId === source.id)));
    expect(expanded.relatedConnections.map(connection => connection.id)).toContain("C-support:v1");
    const text = (result.content as { type: string; text: string }[])[0].text;
    expect(JSON.parse(text)).toEqual(expanded);
  });
  it("keeps raw audit metadata available by source ID but excludes it from bounded reconstruction inputs", async () => {
    const source = bundle.sources[0]; source.provenance = { kind: "chat", payload: { private_debug: "Do not include in evaluator packet", nested: { exact: [1, 2, 3] } } };
    mock.packet.mockResolvedValue({ ...buildEvaluationPacket(bundle), serverAuthority: { receipts: [] } });
    const raw = resultValue<{ source: typeof source }>(await protocol(client => client.callTool({ name: "expand_reasoning", arguments: { project_ref: "project:test", dataset_id: bundle.id, record_id: source.id } })));
    expect(raw.source.provenance!.payload).toEqual(source.provenance.payload);
    const packet = resultValue(await protocol(client => client.callTool({ name: "get_reconstruction_packet", arguments: { project_ref: "project:test", dataset_id: bundle.id } })));
    expect(JSON.stringify(packet)).not.toContain("private_debug"); expect(packet).toHaveProperty("serverAuthority");
  });
  it.each(["actor", "run", "context", "external", "portable-receipt", "server-receipt"])("expands supporting %s identity without granting imported authority", async kind => {
    let recordId: string;
    if (kind === "actor") recordId = bundle.actors[0].id;
    else if (kind === "run") recordId = bundle.runs[0].id;
    else if (kind === "context") recordId = bundle.contexts[0].id;
    else if (kind === "external") { recordId = "outside"; bundle.externalEntryRefs.push({ id: recordId, namespace: { datasetId: "other", workspaceId: "w", projectId: "p" }, entryId: "remote", originalId: "original" }); }
    else {
      recordId = kind;
      const receipt = { id: recordId, action: "adopt" as const, entryId: bundle.entries[0].id, priorReceiptId: null, actorId: "fictional-human", grantId: null, recordedAt: "2026-10-12T12:00:00Z", rationale: "Recorded history", connectionId: null, basis: "authorized_action" as const, sourceSpans: [] };
      if (kind === "portable-receipt") bundle.commitmentReceipts.push(receipt);
      else mock.read.mockResolvedValue({ bundle, head: "head", commitments: [receipt], authorityGrants: [] });
    }
    const value = resultValue(await protocol(client => client.callTool({ name: "expand_reasoning", arguments: { project_ref: "project:test", dataset_id: bundle.id, record_id: recordId } })));
    if (kind === "server-receipt") expect(value.serverAuthority).toMatchObject({ receipt: { id: recordId } });
    else { expect(value.supportingRecord).toMatchObject({ id: recordId }); expect(value.supportingRecordAuthority).toContain("does not confer live authority"); }
  });
  it("does not return a guessed record for an unknown ID", async () => {
    const result = await protocol(client => client.callTool({ name: "expand_reasoning", arguments: { project_ref: "project:test", dataset_id: bundle.id, record_id: "missing" } }));
    expect(result.isError).toBe(true); expect(result.structuredContent).toBeUndefined();
  });
  it("passes historical reconstruction boundaries unchanged", async () => {
    mock.packet.mockResolvedValue(buildEvaluationPacket(bundle, "R2", 3));
    const result = await protocol(client => client.callTool({ name: "get_reconstruction_packet", arguments: { project_ref: "project:test", dataset_id: bundle.id, run_id: "R2", source_cutoff: 3 } }));
    expect(mock.packet).toHaveBeenCalledWith("project:test", bundle.id, "R2", 3, undefined);
    expect(JSON.stringify(resultValue(result))).not.toContain("E-new-test:v1");
  });
});
