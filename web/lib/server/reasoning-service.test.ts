import { describe, expect, it } from "vitest";
import type { SupabaseClient } from "@supabase/supabase-js";
import { upgradeBundle } from "@reasoning/domain";
import type { CommitmentReceipt } from "@reasoning/types";
import { makeDemoBundle } from "../../../prototypes/reasoning/src/fixture";
import type { ProjectRecord } from "@/types/graph";
import { ReasoningService, type StoredReasoning } from "./reasoning-service";

const project = (id: string, parent: string | null = null): ProjectRecord => ({ id, workspace_id: "w", ref: `project:${id}`, name: id, description: "", parent_id: parent, created_at: "2026-01-01T00:00:00Z", updated_at: "2026-01-01T00:00:00Z" });
function stored(projectId: string, inherited = false, scope: string[] = []): StoredReasoning {
  const bundle = upgradeBundle(makeDemoBundle()); bundle.id = `dataset-${projectId}`;
  bundle.namespace = { datasetId: bundle.id, workspaceId: "w", projectId }; bundle.contexts[0].kind = "discussion";
  const source = bundle.sources.find(item => item.id === "S3")!;
  source.role = "record"; source.provenance = { kind: "legacy_record", originalRecord: { table: "nodes", id: "legacy-decision", workspaceId: "w" }, payload: { id: "legacy-decision", workspace_id: "w", project_id: projectId, scope_refs: scope, inheritance_mode: inherited ? "descendants" : "local" } };
  const receipt: CommitmentReceipt = { id: `receipt-${projectId}`, action: "adopt", entryId: "E-commitment:v1", priorReceiptId: null, actorId: "human:owner", grantId: null, recordedAt: "2026-10-06T10:00:00-07:00", rationale: "Recorded approval", connectionId: null, basis: "legacy_approval", sourceSpans: [{ messageId: "S3", start: 0, end: source.content.length }] };
  return { bundle, head: `head-${projectId}`, commitments: [receipt], authorityGrants: [] };
}
class FakeService extends ReasoningService {
  mode = "reasoning_open";
  projectRows = [project("parent"), project("child", "parent")];
  data = new Map([["project:parent", stored("parent", true, ["workflow:writing"])], ["project:child", stored("child")]]);
  constructor() { super({} as SupabaseClient, { workspaceId: "w", actor: { id: "human:owner", name: "Owner", kind: "human" }, isOwner: true, canWrite: true, agent: false }); }
  override async projects() { return structuredClone(this.projectRows); }
  override async readiness() { return { mode: this.mode, ready: this.mode === "reasoning_open" }; }
  override async datasets(ref: string) {
    const value = this.data.get(ref)!;
    return structuredClone({ datasets: [{ id: value.bundle.id, title: value.bundle.title, head: value.head }], grants: value.authorityGrants, commitments: value.commitments, connections: [] });
  }
  override async read(ref: string) { return structuredClone(this.data.get(ref)!); }
}
describe("reasoning service scoped guidance", () => {
  it("selects bounded reconstruction records without weakening grouped connections", async () => {
    const service = new FakeService(), stored = service.data.get("project:child")!;
    const grouped = stored.bundle.connections.find(connection => connection.id === "C-support:v1")!;
    const participants = grouped.participants.map(participant => participant.entryId);
    const full = await service.packet("project:child", stored.bundle.id, undefined, undefined, participants);
    expect(full.connections.map(connection => connection.id)).toContain(grouped.id);
    const partial = await service.packet("project:child", stored.bundle.id, undefined, undefined, participants.slice(0, 1));
    expect(partial.connections.map(connection => connection.id)).not.toContain(grouped.id);
    expect(partial.entries).toHaveLength(1);
    expect(partial).toHaveProperty("selection.omittedConnections");
    await expect(service.packet("project:child", stored.bundle.id, undefined, undefined, ["not-present"])).rejects.toThrow("unavailable");
  });
  it.each(["legacy_open", "read_only", "frozen", "unknown"])("never claims empty guidance is complete before reasoning is open: %s", async mode => {
    const service = new FakeService(); service.mode = mode;
    service.datasets = async () => ({ datasets: [], grants: [], commitments: [], connections: [] });
    const result = await service.brief("project:child");
    expect(result.datasets).toEqual([]); expect(result.complete).toBe(false); expect(result.coverage_complete).toBe(false);
    expect(result.readiness).toEqual({ mode, ready: false }); expect(result.warnings.join(" ")).toContain("not complete authoritative guidance");
    service.mode = "reasoning_open";
    const opened = await service.brief("project:child", result.fingerprint);
    expect(opened.complete).toBe(true); expect(opened.changed).toBe(true);
  });
  it("retains staged records for inspection and rejects readiness changes during retrieval", async () => {
    const service = new FakeService(); service.mode = "frozen";
    const staged = await service.brief("project:child", undefined, { task: "Write", workflow_ref: "workflow:writing" });
    expect(staged.datasets).toHaveLength(2); expect(staged.complete).toBe(false);
    service.mode = "reasoning_open"; let reads = 0;
    service.readiness = async () => ++reads === 1 ? { mode: "reasoning_open", ready: true } : { mode: "frozen", ready: false };
    const changing = await service.brief("project:child", undefined, { task: "Write", workflow_ref: "workflow:writing" });
    expect(changing.complete).toBe(false); expect(changing.warnings.join(" ")).toContain("readiness changed");
  });
  it("preserves ancestor inheritance and requires missing workflow scope before claiming completeness", async () => {
    const service = new FakeService(); const missing = await service.brief("project:child");
    expect(missing.complete).toBe(false); expect(missing.coverage_complete).toBe(true);
    expect(missing.scope_questions).toEqual([{ entry_id: "E-commitment:v1", dataset_id: "dataset-parent", project_ref: "project:parent", dimension: "workflow:", options: ["workflow:writing"] }]);
    const resolved = await service.brief("project:child", undefined, { task: "Write", workflow_ref: "workflow:writing" });
    expect(resolved.complete).toBe(true); expect(resolved.datasets).toHaveLength(2);
    expect(resolved.datasets.find(dataset => dataset.project_ref === "project:parent")).toMatchObject({ inherited: true, entries: [{ source_project_ref: "project:parent", inherited_from_project_id: "parent", scope_refs: ["workflow:writing"] }] });
    expect(resolved.datasets.find(dataset => dataset.project_ref === "project:child")).toMatchObject({ inherited: false });
  });
  it("excludes scope mismatches but does not silently substitute local guidance for a parent", async () => {
    const brief = await new FakeService().brief("project:child", undefined, { task: "Deploy", workflow_ref: "workflow:deploy" });
    expect(brief.complete).toBe(true); expect(brief.datasets).toHaveLength(1);
    expect("excluded" in brief && brief.excluded[0]).toMatchObject({ project_ref: "project:parent", reason: "Outside the selected scope" });
  });
  it("never inherits candidate/imported receipts or native local authority from a parent", async () => {
    const service = new FakeService(), parent = service.data.get("project:parent")!;
    parent.bundle.commitmentReceipts = [...parent.commitments]; parent.commitments = [];
    expect((await service.brief("project:child")).datasets).toHaveLength(1);
    parent.commitments = [{ ...parent.bundle.commitmentReceipts[0], basis: "authorized_action" }];
    expect((await service.brief("project:child")).datasets).toHaveLength(1);
  });
  it("keeps inherited policy through authorized revisions and preserves the original receipt", async () => {
    const service = new FakeService(), parent = service.data.get("project:parent")!, original = structuredClone(parent.commitments[0]);
    parent.commitments.push({ ...original, id: "revision", action: "revise", entryId: "E-objection:v1", priorReceiptId: original.id, basis: "authorized_action", actorId: "agent:delegated", recordedAt: "2026-10-09T11:00:00-07:00" });
    const brief = await service.brief("project:child", undefined, { task: "Write", workflow_ref: "workflow:writing" });
    expect(brief.datasets.find(dataset => dataset.project_ref === "project:parent")).toMatchObject({ commitments: [{ id: "revision" }], entries: [{ inheritance_mode: "descendants", scope_refs: ["workflow:writing"] }] });
    expect(parent.commitments[0]).toEqual(original);
  });
  it("fingerprints authority, ancestor uncertainties and selected scope", async () => {
    const service = new FakeService(), scope = { task: "Write", workflow_ref: "workflow:writing" };
    const initial = await service.brief("project:child", undefined, scope);
    expect((await service.brief("project:child", initial.fingerprint, scope)).changed).toBe(false);
    service.data.get("project:parent")!.bundle.runs.at(-1)!.uncertainties.push("New unresolved concern");
    expect((await service.brief("project:child", initial.fingerprint, scope)).changed).toBe(true);
    expect((await service.brief("project:child", initial.fingerprint, { task: "Deploy", workflow_ref: "workflow:deploy" })).changed).toBe(true);
  });
  it("marks concurrent changes incomplete rather than returning a false complete read", async () => {
    const service = new FakeService(), originalRead = service.read.bind(service);
    service.read = async ref => { const result = await originalRead(ref); service.data.get(ref)!.head += "-changed"; return result; };
    const brief = await service.brief("project:child", undefined, { task: "Write", workflow_ref: "workflow:writing" });
    expect(brief.complete).toBe(false); expect(brief.coverage_complete).toBe(false); expect(brief.warnings.join(" ")).toContain("changed during retrieval");
  });
  it("returns verified server authority separately and excludes later actions from historical packets", async () => {
    const service = new FakeService(), current = service.data.get("project:child")!, initial = current.commitments[0];
    current.commitments.push({ ...initial, id: "later", action: "revise", entryId: "E-objection:v1", priorReceiptId: initial.id, recordedAt: "2026-10-12T11:00:00-07:00", basis: "authorized_action", sourceSpans: [] });
    const latest = await service.packet("project:child", current.bundle.id);
    expect(latest.serverAuthority.receipts.map(receipt => receipt.id)).toContain("later");
    const historical = await service.packet("project:child", current.bundle.id, "R2", 3);
    expect(historical.serverAuthority.receipts.map(receipt => receipt.id)).not.toContain("later");
    expect(historical.serverAuthority.receipts.map(receipt => receipt.id)).toContain(initial.id);
    expect(historical.commitmentReceipts).toEqual([]);
  });
  it("retains authenticated capture attribution separately from portable expressing and recording claims", async () => {
    const service = new FakeService(), value = service.data.get("project:child")!;
    value.captureMetadata = { "E-commitment:v1": { actorId: "agent:actual-recorder", recordedAt: "2026-10-12T20:00:00Z" }, "E-new-test:v1": { actorId: "agent:later", recordedAt: "2026-10-12T21:00:00Z" } };
    const packet = await service.packet("project:child", value.bundle.id, "R2", 3);
    expect(packet.serverCapture["E-commitment:v1"]).toEqual(value.captureMetadata["E-commitment:v1"]);
    expect(packet.serverCapture).not.toHaveProperty("E-new-test:v1");
    expect(packet.entries.find(entry => entry.id === "E-commitment:v1")!.provenance).toMatchObject({ expressedBy: "fictional-human", recordedBy: "demo-recorder" });
  });
});
