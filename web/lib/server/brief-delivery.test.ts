import { describe, expect, it, vi } from "vitest";
import { deliverTaskBrief, serializedBriefBytes, type BriefDeliveryEdge, type BriefDeliveryNode, type DeliverableBrief } from "./brief-delivery";
import { briefScopeKey, type BriefBaselineManifest, type BriefBaselineStore, type StoredBriefBaseline } from "./brief-baseline";
import { recordHash, type BriefRequest } from "./task-brief";
import type { ProjectRecord } from "@/types/graph";

const now = Date.parse("2026-10-04T12:00:00Z");
const project: ProjectRecord = { id: "p", workspace_id: "w", ref: "project:test", name: "Test", description: "Project metadata".repeat(20), parent_id: null, created_at: "t1", updated_at: "t1" };
const node = (id: string, patch: Partial<BriefDeliveryNode> = {}): BriefDeliveryNode => ({
  id, type: "Decision", status: "active", title: id, summary: "Summary is not the operative body", body: "Do this only for the confirmed target. Exception: retain existing authorized behavior.",
  scope_type: "project", scope_refs: ["project:test"], confidence: "high", reversibility: "medium", consequence: "high",
  reconsider_conditions: ["New evidence contradicts the stated premise"], metadata: { source_ref: "thread:one", evidence_summary: "Measured test result" },
  created_at: "t1", updated_at: "t2", ratified_at: "t2", project_id: "p", inheritance_mode: "local", source_project: project,
  ...patch,
});
const edge = (id: string, source_id: string, target_id: string, edge_type = "supports", patch: Partial<BriefDeliveryEdge> = {}): BriefDeliveryEdge => ({
  id, source_id, target_id, edge_type, summary: "Why this relation holds", metadata: { source_ref: "thread:one" }, created_at: "t1", ...patch,
});
const request: BriefRequest = { project_ref: "project:test", task: "Assess architecture", workflow_ref: "workflow:review", delivery: "compact" };
function brief(records = [node("D-one")], relationships: BriefDeliveryEdge[] = [], patch: Partial<DeliverableBrief> = {}): DeliverableBrief {
  return {
    project_ref: request.project_ref, fingerprint: recordHash({ records, relationships }), complete: true, checked_at: "now", changed: null,
    groups: { active_decisions: records.filter(n => n.type === "Decision" && n.status === "active"),
      accepted_assumptions: records.filter(n => n.type === "Assumption" && n.status === "accepted"),
      observations: records.filter(n => n.status === "observation"), unratified: records.filter(n => ["candidate", "proposed"].includes(n.status)),
      history: records.filter(n => ["superseded", "invalidated", "rejected", "archived"].includes(n.status)) },
    scope_questions: [], warnings: [], projects: [project], relationships, supersession_edges: relationships.filter(e => e.edge_type === "supersedes"),
    coverage: { project_nodes: records.length, returned_nodes: records.length, excluded_by_scope: 0 },
    record_versions: Object.fromEntries(records.map(n => [n.id, { hash: recordHash(n), updated_at: n.updated_at }])), ...patch,
  };
}
function storage() {
  const records = new Map<string, StoredBriefBaseline>();
  let next = 0;
  const store: BriefBaselineStore = {
    get: vi.fn(async (token, _project, scope) => { const value = records.get(token); return value?.manifest.scope_key === scope ? value ?? null : null; }),
    save: vi.fn(async (_project, _scope, manifest) => {
      const token = `00000000-0000-4000-8000-${String(++next).padStart(12, "0")}`;
      const expires_at = new Date(now + 3600000).toISOString();
      records.set(token, { token, expires_at, manifest });
      return { token, expires_at };
    }),
  };
  return { store, records };
}
const options = { workspaceId: "w", principalId: "connection:actor", now };
const groups = (result: Record<string, unknown>) => result.groups as Record<string, BriefDeliveryNode[]>;
const token = (result: Record<string, unknown>) => (result.baseline as { token: string }).token;

describe("bounded brief delivery", () => {
  it("deduplicates only repeated metadata while retaining every full record and reconstructing originals", async () => {
    const records = [node("D-one"), node("D-two"), node("A-premise", { type: "Assumption", status: "accepted" }),
      node("T-finding", { type: "Attempt", status: "observation", body: "Consequential unlinked finding".repeat(40) }),
      node("D-old", { status: "superseded" }), node("D-maybe", { status: "candidate", ratified_at: null }),
      node("T-unknown", { type: "Attempt", status: "observation", source_project: null })];
    const input = brief(records, [edge("E", "D-one", "D-old", "supersedes")]);
    const full = await deliverTaskBrief(input, { ...request, delivery: "full" }, options);
    const deduplicated = await deliverTaskBrief(input, { ...request, delivery: "deduplicated" }, options);
    expect(deduplicated).toMatchObject({ delivery: "deduplicated", complete: true, guidance_complete: true, archive_complete: true, index_complete: true, index_omitted_count: 0 });
    const projects = deduplicated.projects as ProjectRecord[];
    const reconstructed = Object.fromEntries(Object.entries(groups(deduplicated)).map(([group, values]) => [group, values.map(value => {
      const { source_project_id, ...rest } = value;
      return source_project_id ? { ...rest, source_project: projects.find(p => p.id === source_project_id) } : value;
    })]));
    expect(reconstructed).toEqual(groups(full));
    expect(deduplicated.relationships).toEqual(full.relationships);
    expect(deduplicated.record_versions).toEqual(full.record_versions);
    expect((deduplicated.coverage as { optional_nodes_omitted: number }).optional_nodes_omitted).toBe(0);
    expect(serializedBriefBytes(deduplicated)).toBeLessThan(serializedBriefBytes(full));
    const { store } = storage();
    const first = await deliverTaskBrief(input, { ...request, delivery: "deduplicated" }, { ...options, baselineStore: store });
    const unchanged = await deliverTaskBrief(input, { ...request, delivery: "deduplicated", baseline_token: token(first), baseline_confirmed: true }, { ...options, baselineStore: store });
    expect(unchanged.delivery).toBe("unchanged");
    const bounded = await deliverTaskBrief(input, { ...request, delivery: "deduplicated", max_response_bytes: 1024 }, options);
    expect(bounded).toMatchObject({ delivery: "index", complete: false });
  });

  it("keeps authoritative bodies, exceptions, scope and provenance while deduplicating project metadata", async () => {
    const input = brief([node("D-one"), node("A-premise", { type: "Assumption", status: "accepted" }), node("T-unrelated", { type: "Attempt", status: "observation", body: "Routine log".repeat(200) })]);
    const result = await deliverTaskBrief(input, request, options);
    expect(result).toMatchObject({ delivery: "compact", complete: true, guidance_complete: true, archive_complete: false });
    expect(groups(result).active_decisions[0]).toMatchObject({ body: input.groups.active_decisions[0].body, scope_refs: ["project:test"], source_project_id: "p", metadata: input.groups.active_decisions[0].metadata });
    expect(groups(result).active_decisions[0]).not.toHaveProperty("source_project");
    expect(groups(result).accepted_assumptions).toHaveLength(1);
    expect(groups(result).observations).toEqual([]);
    expect(result).not.toHaveProperty("record_versions");
    const full = await deliverTaskBrief(input, { ...request, delivery: "full" }, options);
    expect(groups(full).observations).toHaveLength(1);
    expect(serializedBriefBytes(result)).toBeLessThan(serializedBriefBytes(full));
  });

  it("protects contradictory evidence and dependency chains without promoting proposals", async () => {
    const evidence = node("T-counter", { type: "Attempt", status: "observation" });
    const candidate = node("D-candidate", { status: "candidate", ratified_at: null });
    const input = brief([node("D-one"), evidence, candidate], [edge("E-counter", "T-counter", "D-one", "contradicts"), edge("E-replacement", "D-candidate", "D-one", "supersedes", { metadata: { proposed: true } })]);
    const result = await deliverTaskBrief(input, { ...request, max_linked_nodes: 0 }, options);
    expect(groups(result).observations[0]).toEqual(expect.objectContaining({ id: "T-counter", status: "observation" }));
    expect(groups(result).unratified[0]).toEqual(expect.objectContaining({ id: "D-candidate", status: "candidate" }));
    expect(groups(result).active_decisions.map(n => n.id)).toEqual(["D-one"]);
    expect(result.relationships).toHaveLength(2);
    expect(result.complete).toBe(true);
  });

  it("marks missing or unresolved material endpoints incomplete and cannot mint a baseline", async () => {
    const { store } = storage();
    const input = brief([node("D-one")], [edge("E", "T-hidden", "D-one", "contradicts")]);
    const result = await deliverTaskBrief(input, request, { ...options, baselineStore: store });
    expect(result).toMatchObject({ complete: false, guidance_complete: false, expansion_required: ["T-hidden"] });
    expect(store.save).not.toHaveBeenCalled();
    const scoped = await deliverTaskBrief(brief(undefined, [], { scope_questions: [{ node_id: "D-other", dimension: "target:", options: ["target:other"] }] }), request, { ...options, baselineStore: store });
    expect(scoped.complete).toBe(false);
    expect(store.save).not.toHaveBeenCalled();
  });

  it("returns an explicit bounded index rather than truncating an oversized required body", async () => {
    const { store } = storage();
    const input = brief([node("D-one", { body: "🧠".repeat(5000) })]);
    const result = await deliverTaskBrief(input, { ...request, max_response_bytes: 2048 }, { ...options, baselineStore: store });
    expect(result).toMatchObject({ delivery: "index", complete: false, guidance_complete: false, expansion_required: ["D-one"] });
    expect(result).not.toHaveProperty("groups");
    expect(serializedBriefBytes(result)).toBeLessThanOrEqual(2048);
    expect(store.save).not.toHaveBeenCalled();
  });

  it("caps optional linked context and retains its material counter-evidence as one indivisible bundle", async () => {
    const records = [node("D-one"), ...["T-root", "T-a", "T-b", "T-c"].map(id => node(id, { type: "Attempt", status: "observation" }))];
    const input = brief(records, [edge("E-root", "T-root", "D-one"), edge("E-a", "T-a", "T-root"), edge("E-b", "T-b", "T-root"), edge("E-c", "T-c", "T-a", "contradicts")]);
    const result = await deliverTaskBrief(input, { ...request, max_linked_nodes: 1 }, options);
    expect(groups(result).observations.map(n => n.id)).toEqual(["T-root", "T-b"]);
    const expanded = await deliverTaskBrief(input, { ...request, max_linked_nodes: 2 }, options);
    expect(groups(expanded).observations.map(n => n.id)).toEqual(["T-root", "T-a", "T-c"]);
    expect(expanded.relationships).toHaveLength(3);
  });

  it("preserves buried exceptions in direct supporting evidence despite optional limits", async () => {
    const input = brief([node("D-one"), node("T-proof", { type: "Attempt", status: "observation", summary: "Supported outcome", body: "Works normally. Exception: never apply to legacy records." })], [edge("E-proof", "T-proof", "D-one")]);
    const result = await deliverTaskBrief(input, { ...request, max_linked_nodes: 0 }, options);
    expect(groups(result).observations[0].body).toContain("never apply to legacy records");
    expect(result.complete).toBe(true);
  });

  it("keeps unlinked findings discoverable without presenting summary previews as guidance", async () => {
    const input = brief([node("D-one"), node("T-canary", { type: "Attempt", status: "observation", title: "Canary architecture finding", summary: "A prior experiment found the distinctive condition", body: "Full detailed evidence" })]);
    const result = await deliverTaskBrief(input, request, options);
    expect(result.record_index).toEqual([expect.objectContaining({ id: "T-canary", title: "Canary architecture finding", partial: true, source_ref: "thread:one" })]);
    expect(result.index_complete).toBe(true);
    expect(result.index_instructions).toContain("Expand a relevant ID");
    expect(result.guidance_complete).toBe(true);
  });

  it("pre-expands task evidence only when requested, preserving exact evidence and observation authority", async () => {
    const finding = node("T-prefetch", { type: "Attempt", status: "observation", title: "Shard lease measurements", summary: "A timed lease experiment.",
      body: "Lease expires after exactly 137 seconds. Exception: old-format leases need 29 seconds. Ticket NF-8123.", ratified_at: null });
    const input = brief([node("D-one"), finding]);
    const taskRequest = { ...request, task: "Inspect shard lease evidence and report the exact expiry and exception." };
    const incumbent = await deliverTaskBrief(input, taskRequest, options);
    expect(groups(incumbent).observations).toEqual([]);
    expect(await deliverTaskBrief(input, { ...taskRequest, max_task_evidence_nodes: 0 }, options)).toEqual(incumbent);
    const result = await deliverTaskBrief(input, { ...taskRequest, max_task_evidence_nodes: 1 }, options);
    expect(groups(result).observations).toEqual([expect.objectContaining({ id: finding.id, body: finding.body, status: "observation", ratified_at: null, metadata: finding.metadata })]);
    expect(groups(result).active_decisions).toEqual(groups(incumbent).active_decisions);
    expect(result.relationships).toEqual([]);
    expect(result.record_index).toEqual([]);
    const full = await deliverTaskBrief(input, { ...taskRequest, delivery: "full" }, options);
    expect(await deliverTaskBrief(input, { ...taskRequest, delivery: "full", max_task_evidence_nodes: 1 }, options)).toEqual(full);
  });

  it("does not prefetch generic, body-only, partial-token, retired, proposed or already-linked matches", async () => {
    const observation: Partial<BriefDeliveryNode> = { type: "Attempt", status: "observation", summary: "", body: "Unrelated fixture." };
    const records = [node("D-one"),
      node("T-generic", { ...observation, title: "Current investigation evidence report" }),
      node("T-body-only", { ...observation, title: "Routine operations", body: "The quartz lease detail is in this body." }),
      node("T-substring", { ...observation, title: "quartzite leaseholder" }),
      node("T-retired", { ...observation, status: "archived", title: "Quartz lease" }),
      node("T-proposed", { type: "Decision", status: "candidate", title: "Quartz lease" }),
      node("T-linked", { ...observation, title: "Quartz lease" }),
      node("T-peer", { ...observation, title: "Unrelated peer" }),
      node("T-external", { ...observation, title: "Quartz lease" })];
    const input = brief(records, [edge("E", "T-linked", "T-peer", "contradicts")], {
      reference_edges: [{ id: "E-ref", source_id: "T-external", target_id: "T-outside", edge_type: "supports", proposed: true }],
    });
    const result = await deliverTaskBrief(input, { ...request, task: "Inspect current quartz lease investigation evidence report", max_task_evidence_nodes: 8, max_linked_nodes: 0 }, options);
    expect(groups(result).observations).toEqual([]);
    expect(groups(result).unratified).toEqual([]);
    expect(groups(result).history).toEqual([]);
    expect(result.record_index).toHaveLength(records.length - 1);
    expect(result.relationships).toEqual([]);
    expect(result.reference_edges).toEqual(input.reference_edges);
  });

  it("bounds task evidence by requested count and bytes without clipping bodies or losing index discovery", async () => {
    const records = Array.from({ length: 12 }, (_, i) => node(`T-${String(i).padStart(2, "0")}`, {
      type: "Attempt", status: "observation", title: "Quartz lease", summary: "Timing finding", body: i === 0 ? "Oversized exact evidence ".repeat(3000) : "Expiry is exactly 137 seconds.",
    }));
    const input = brief([node("D-one"), ...records]);
    const taskRequest = { ...request, task: "Inspect quartz lease timing", max_task_evidence_nodes: 1, max_linked_nodes: 0, max_response_bytes: 6000 };
    const result = await deliverTaskBrief(input, taskRequest, options);
    expect(groups(result).observations.map(n => n.id)).toEqual(["T-01"]);
    expect(groups(result).observations[0].body).toBe(records[1].body);
    expect(result.record_index).toContainEqual(expect.objectContaining({ id: "T-00", partial: true }));
    expect(result).toMatchObject({ complete: true, guidance_complete: true });
    expect(serializedBriefBytes(result)).toBeLessThanOrEqual(6000);
    const capped = await deliverTaskBrief(brief([node("D-one"), ...records.slice(1)]), { ...taskRequest, max_task_evidence_nodes: 99, max_response_bytes: 100000 }, options);
    expect(groups(capped).observations).toHaveLength(8);
    const one = await deliverTaskBrief(brief([node("D-one"), ...records.slice(1)]), { ...taskRequest, max_response_bytes: 100000 }, options);
    expect(groups(one).observations).toHaveLength(1);
  });

  it("reports a truncated discovery index separately from required guidance completeness", async () => {
    const input = brief([node("D-one"), ...Array.from({ length: 20 }, (_, i) => node(`T-${i}`, { type: "Attempt", status: "observation", summary: "Long finding summary ".repeat(20) }))]);
    const result = await deliverTaskBrief(input, { ...request, max_response_bytes: 6000 }, options);
    expect(result).toMatchObject({ delivery: "compact", complete: true, guidance_complete: true, index_complete: false });
    expect(result.index_omitted_count).toBeGreaterThan(0);
    expect(serializedBriefBytes(result)).toBeLessThanOrEqual(6000);
  });
  it("carries external reference notices without treating them as missing governing guidance", async () => {
    const reference = { id: "E-cross", source_id: "D-one", target_id: "D-foreign", edge_type: "supports", proposed: false };
    const input = brief(undefined, [], { reference_edges: [reference], reference_notice: "References do not enable inheritance." });
    const result = await deliverTaskBrief(input, request, options);
    expect(result.reference_edges).toEqual([reference]);
    expect(result.relationships).toEqual([]);
    expect(result).toMatchObject({ complete: true, guidance_complete: true, expansion_required: [] });
    expect(Object.values(groups(result)).flat().map(n => n.id)).toEqual(["D-one"]);
  });
});

describe("retained baseline delivery", () => {
  it("requires explicit possession and a retained scope-bound baseline before omitting unchanged records", async () => {
    const { store } = storage();
    const input = brief();
    const first = await deliverTaskBrief(input, request, { ...options, baselineStore: store });
    const baseline_token = token(first);
    const noConfirmation = await deliverTaskBrief(input, { ...request, baseline_token }, { ...options, baselineStore: store });
    expect(noConfirmation.delivery).toBe("compact");
    const fingerprintOnly = await deliverTaskBrief(input, { ...request, previous_fingerprint: input.fingerprint }, { ...options, baselineStore: store });
    expect(fingerprintOnly.delivery).toBe("compact");
    const unchanged = await deliverTaskBrief(input, { ...request, baseline_token, baseline_confirmed: true }, { ...options, baselineStore: store });
    expect(unchanged).toMatchObject({ delivery: "unchanged", complete: true, changed: false });
    expect(unchanged).not.toHaveProperty("groups");
    expect(serializedBriefBytes(unchanged)).toBeLessThan(serializedBriefBytes(first));
  });

  it.each([
    ["workspace", { ...options, workspaceId: "elsewhere" }, request],
    ["principal", { ...options, principalId: "other:actor" }, request],
    ["workflow", options, { ...request, workflow_ref: "workflow:other" }],
    ["target", options, { ...request, target_ref: "target:other" }],
    ["context", options, { ...request, context_refs: ["trial:new"] }],
    ["task", options, { ...request, task: "Different investigation" }],
    ["delivery controls", options, { ...request, max_linked_nodes: 0 }],
    ["task evidence limit", options, { ...request, max_task_evidence_nodes: 1 }],
  ])("does not reuse a baseline across %s", async (_label, changedOptions, changedRequest) => {
    const { store } = storage();
    const first = await deliverTaskBrief(brief(), request, { ...options, baselineStore: store });
    const result = await deliverTaskBrief(brief(), { ...changedRequest, baseline_token: token(first), baseline_confirmed: true }, { ...changedOptions, baselineStore: store });
    expect(result.delivery).toBe("compact");
    expect(groups(result).active_decisions).toHaveLength(1);
  });

  it("treats expired, missing and failed baseline storage as a full-payload fallback", async () => {
    const { store, records } = storage();
    const first = await deliverTaskBrief(brief(), request, { ...options, baselineStore: store });
    records.get(token(first))!.expires_at = new Date(now - 1).toISOString();
    const continued = { ...request, baseline_token: token(first), baseline_confirmed: true, allow_delta: true };
    expect((await deliverTaskBrief(brief(), continued, { ...options, baselineStore: store })).delivery).toBe("compact");
    expect((await deliverTaskBrief(brief(), continued, options)).delivery).toBe("compact");
    const failed: BriefBaselineStore = { get: async () => { throw new Error("offline"); }, save: async () => { throw new Error("offline"); } };
    const result = await deliverTaskBrief(brief(), continued, { ...options, baselineStore: failed });
    expect(result).toMatchObject({ delivery: "compact", complete: true });
    expect(result).not.toHaveProperty("baseline");
    expect(result.baseline_warnings).toHaveLength(2);
  });

  it("returns changed nodes and relationship tombstones, including group/ownership/inheritance changes", async () => {
    const { store } = storage();
    const original = brief([node("D-one"), node("A-gone", { type: "Assumption", status: "accepted" }), node("D-parent", { inherited_from_project_id: "parent", inheritance_mode: "descendants" })], [edge("E-old", "A-gone", "D-one", "based_on_assumption")]);
    const first = await deliverTaskBrief(original, request, { ...options, baselineStore: store });
    const current = brief([node("D-one", { body: "Revised rule", project_id: "parent", inherited_from_project_id: "parent", inheritance_mode: "descendants" }), node("T-counter", { type: "Attempt", status: "observation" })], [edge("E-new", "T-counter", "D-one", "contradicts")]);
    const delta = await deliverTaskBrief(current, { ...request, baseline_token: token(first), baseline_confirmed: true, allow_delta: true }, { ...options, baselineStore: store });
    expect(delta).toMatchObject({ delivery: "delta", changed: true, removed_node_ids: ["D-parent", "A-gone"], removed_relationship_ids: ["E-old"] });
    expect(groups(delta).active_decisions[0]).toMatchObject({ id: "D-one", body: "Revised rule", inherited_from_project_id: "parent" });
    expect(groups(delta).observations[0].id).toBe("T-counter");
    expect(delta.relationships).toEqual([edge("E-new", "T-counter", "D-one", "contradicts")]);
    expect(token(delta)).not.toBe(token(first));
  });

  it("invalidates on relationship state changes and hierarchy metadata without node text edits", async () => {
    const { store } = storage();
    const records = [node("D-one"), node("D-proposed", { status: "candidate", ratified_at: null })];
    const initial = brief(records, [edge("E", "D-proposed", "D-one", "supersedes", { metadata: { proposed: true } })]);
    const first = await deliverTaskBrief(initial, request, { ...options, baselineStore: store });
    const changedProject = { ...project, parent_id: "new-parent" };
    const current = brief(records.map(record => ({ ...record, source_project: changedProject })), [edge("E", "D-proposed", "D-one", "supersedes", { metadata: { proposed: false } })], { projects: [changedProject] });
    const delta = await deliverTaskBrief(current, { ...request, baseline_token: token(first), baseline_confirmed: true, allow_delta: true }, { ...options, baselineStore: store });
    expect(delta).toMatchObject({ delivery: "delta", metadata_changed: true });
    expect(Object.values(groups(delta)).flat()).toEqual([]);
    expect(delta.relationships).toEqual(current.relationships);
    expect(delta.projects).toEqual(current.projects);
  });

  it("persists hashes and IDs only and normalizes context order in scope bindings", async () => {
    const { store, records } = storage();
    const first = await deliverTaskBrief(brief(), request, { ...options, baselineStore: store });
    const manifest: BriefBaselineManifest = records.get(token(first))!.manifest;
    expect(JSON.stringify(manifest)).not.toContain("Exception:");
    expect(manifest.nodes["D-one"].hash).toMatch(/^[a-f0-9]{64}$/);
    expect(briefScopeKey("w", "p", { ...request, context_refs: ["artifact:a", "trial:b"] }))
      .toBe(briefScopeKey("w", "p", { ...request, context_refs: ["trial:b", "artifact:a", "artifact:a"] }));
    expect(briefScopeKey("w", "p", request)).toBe(briefScopeKey("w", "p", { ...request, max_task_evidence_nodes: 0 }));
    expect(briefScopeKey("w", "p", { ...request, max_task_evidence_nodes: 1 })).not.toBe(briefScopeKey("w", "p", { ...request, max_task_evidence_nodes: 2 }));
  });
});
