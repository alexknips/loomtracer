import { describe, expect, it } from "vitest";
import { buildTaskBrief, readStableSnapshot, scopedSnapshot, snapshotFingerprint, type BriefSnapshot } from "./task-brief";
import type { GraphNodeRecord, GraphEdgeRecord } from "@/types/graph";

const node = (id: string, patch: Partial<GraphNodeRecord> = {}): GraphNodeRecord => ({
  id, workspace_id: "w", type: "Decision", status: "active", title: "Unrelated wording", summary: "Constraint", body: "Full evidence",
  scope_type: "project", scope_refs: ["project:test"], confidence: "high", reversibility: null, consequence: null,
  reconsider_conditions: ["User changes direction"], metadata: { source_ref: "source:one" },
  created_by_actor_id: "author", ratified_by_actor_id: "owner", ratified_by_user_id: "owner", created_at: "t1", updated_at: "t2", ratified_at: "t2", ...patch,
});
const snap = (nodes: GraphNodeRecord[], edges: GraphEdgeRecord[] = []): BriefSnapshot => ({ nodes, edges, complete: true, warnings: [] });
const request = { project_ref: "project:test", task: "Write a manor adventure" };

describe("task briefing", () => {
  it("includes every applicable active decision without keyword or top-k loss", () => {
    const brief = buildTaskBrief(snap(Array.from({ length: 30 }, (_, i) => node(`D-${i}`))), "w", request, "now");
    expect(brief.groups.active_decisions).toHaveLength(30);
    expect(brief.complete).toBe(true);
  });
  it("enforces exact workspace and project boundaries", () => {
    const snapshot = snap([node("ok"), node("other-w", { workspace_id: "other" }), node("other-p", { scope_refs: ["project:test-extra"] })]);
    expect(buildTaskBrief(snapshot, "w", request, "now").groups.active_decisions.map(n => n.id)).toEqual(["ok"]);
  });
  it("separates scope alternatives and requests missing scope rather than guessing", () => {
    const snapshot = snap([node("global"), node("new", { scope_refs: ["project:test", "target:new", "trial:one"] }), node("hush", { scope_refs: ["project:test", "artifact:hush"] })]);
    const ambiguous = buildTaskBrief(snapshot, "w", request, "now");
    expect(ambiguous.groups.active_decisions.map(n => n.id)).toEqual(["global"]);
    expect(ambiguous.scope_questions.map(q => q.dimension)).toContain("target:");
    const scoped = buildTaskBrief(snapshot, "w", { ...request, target_ref: "target:new", context_refs: ["trial:one", "artifact:manor"] }, "now");
    expect(scoped.groups.active_decisions.map(n => n.id)).toEqual(["global", "new"]);
    expect(scoped.scope_questions).toEqual([]);
  });
  it("never promotes a newer candidate or a proposed supersession", () => {
    const snapshot = snap([node("old"), node("new", { status: "candidate", updated_at: "t9" }), node("past", { status: "superseded" }), node("maybe", { type: "Assumption", status: "accepted" })]);
    const brief = buildTaskBrief(snapshot, "w", request, "now");
    expect(brief.groups.active_decisions.map(n => n.id)).toEqual(["old"]);
    expect(brief.groups.unratified.map(n => n.id)).toEqual(["new"]);
    expect(brief.groups.history.map(n => n.id)).toEqual(["past"]);
    expect(brief.groups.accepted_assumptions.map(n => n.id)).toEqual(["maybe"]);
  });
  it("fingerprints the entire project, including newly approved records and scope changes", () => {
    const before = snap([node("first")]);
    const fingerprint = snapshotFingerprint(before);
    const after = buildTaskBrief(snap([...before.nodes, node("added")]), "w", { ...request, previous_fingerprint: fingerprint }, "now");
    expect(after.changed).toBe(true);
    expect(snapshotFingerprint(snap([node("first", { scope_refs: ["project:test", "artifact:elsewhere"] })]))).not.toBe(fingerprint);
    expect(snapshotFingerprint(scopedSnapshot(snap([...before.nodes, node("foreign", { scope_refs: ["project:other"] })]), "w", "project:test"))).toBe(fingerprint);
  });
  it("detects effective supersession changes and is stable under row/key order", () => {
    const edge: GraphEdgeRecord = { id: "e", workspace_id: "w", source_id: "new", target_id: "old", edge_type: "supersedes", summary: "reason", metadata: { proposed: true }, created_at: "t3" };
    const a = snap([node("old"), node("new")], [edge]);
    expect(snapshotFingerprint(a)).toBe(snapshotFingerprint(snap([...a.nodes].reverse(), [edge])));
    expect(snapshotFingerprint(a)).not.toBe(snapshotFingerprint(snap(a.nodes, [{ ...edge, metadata: { proposed: false } }])));
  });
  it("retries changed reads and fails visibly on unstable or truncated reads", async () => {
    const versions = [snap([node("a")]), snap([node("a"), node("b")]), snap([node("a"), node("b")])];
    expect((await readStableSnapshot(async () => versions.shift()!)).complete).toBe(true);
    let i = 0;
    expect((await readStableSnapshot(async () => snap([node(String(i++))]))).complete).toBe(false);
    const partial = { ...snap([node("a")]), complete: false, warnings: ["row cap"] };
    const result = await readStableSnapshot(async () => partial);
    expect(result.complete).toBe(false);
    expect(buildTaskBrief(result, "w", { ...request, previous_fingerprint: "old" }, "now").changed).toBeNull();
  });
  it("propagates source read failure rather than reporting a cleared plan", async () => {
    await expect(readStableSnapshot(async () => { throw new Error("offline"); })).rejects.toThrow("offline");
  });
  it("keeps cross-project references separate from guidance and fingerprints changes", () => {
    const link: GraphEdgeRecord = { id: "cross", workspace_id: "w", source_id: "local", target_id: "foreign", edge_type: "supports", summary: "Related experiment", metadata: { proposed: true }, created_at: "t1" };
    const input = snap([node("local"), node("foreign", { scope_refs: ["project:other"] })], [link]);
    const scoped = scopedSnapshot(input, "w", request.project_ref, true);
    const reference = { id: "cross", workspace_id: "w", source_id: "local", target_id: "foreign", edge_type: "supports", proposed: true };
    expect(scoped.edges).toEqual([]);
    expect(scoped.reference_edges).toEqual([reference]);
    const result = buildTaskBrief(scoped, "w", request, "now");
    expect(result.relationships).toEqual([]);
    expect(result.reference_edges).toEqual([reference]);
    expect(result.groups.active_decisions.map(n => n.id)).toEqual(["local"]);
    expect(result.complete).toBe(true);
    expect(result.reference_notice).toContain("do not enable inheritance");
    const after = scopedSnapshot({ ...input, edges: [] }, "w", request.project_ref, true);
    expect(snapshotFingerprint(scoped)).not.toBe(snapshotFingerprint(after));
    expect(JSON.stringify(result.reference_edges)).not.toContain("Related experiment");
  });
  it("omits external reference identity, prose and fingerprints unless explicitly opted in", () => {
    const link: GraphEdgeRecord = { id: "FOREIGN-EDGE", workspace_id: "w", source_id: "local", target_id: "FOREIGN-NODE", edge_type: "supports", summary: "SPARK-FLUX", metadata: { evidence: "SPARK-FLUX", proposed: true }, created_at: "t1" };
    const input = snap([node("local"), node("FOREIGN-NODE", { scope_refs: ["project:other"], body: "SPARK-FLUX" })], [link]);
    const scoped = scopedSnapshot(input, "w", request.project_ref);
    const result = buildTaskBrief(input, "w", request, "now");
    expect(JSON.stringify(result)).not.toMatch(/FOREIGN|SPARK-FLUX/);
    expect(snapshotFingerprint(scoped)).toBe(snapshotFingerprint(scopedSnapshot({ ...input, edges: [] }, "w", request.project_ref)));
    const candidate = scopedSnapshot(input, "w", request.project_ref, true);
    const disabled = scopedSnapshot(candidate, "w", request.project_ref, false);
    expect(disabled).not.toHaveProperty("reference_edges");
    expect(disabled).not.toHaveProperty("include_reference_edges");
    expect(snapshotFingerprint(disabled)).toBe(snapshotFingerprint(scoped));
  });
});
