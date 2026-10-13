import type { BriefRequest, BriefSnapshot } from "../lib/server/task-brief";
import type { GraphNodeRecord, GraphEdgeRecord } from "../types/graph";

// Entirely synthetic. Never seed these records into a real graph.
export const SUITE_VERSION = "decision-continuity-v1";
export const WORKSPACE = "benchmark-workspace";
export const TIME = "2026-01-01T00:00:00.000Z";
export const PROJECT = "project:bench-echeasea";
export type Action = "proceed" | "clarify" | "blocked" | "revise";
export interface Expectation {
  active: string[]; unratified?: string[]; history?: string[];
  action: Action; ruleset: "dnd-5.5e" | "daggerheart" | null;
  complete?: boolean; changed?: boolean | null; error?: boolean;
}
export interface Stage {
  snapshot: BriefSnapshot; expected: Expectation; failure?: boolean;
}
export interface Fixture {
  id: string; name: string; request: BriefRequest; stages: Stage[];
}

export function node(id: string, body: string, patch: Partial<GraphNodeRecord> = {}): GraphNodeRecord {
  return {
    id, workspace_id: WORKSPACE, type: "Decision", status: "active",
    title: "Writing rules", summary: body, body, scope_type: "project", scope_refs: [PROJECT],
    confidence: "high", reversibility: "high", consequence: "medium",
    reconsider_conditions: [], metadata: { source_ref: `fixture:${id}` },
    created_by_actor_id: "fixture-writer", ratified_by_actor_id: "fixture-owner",
    ratified_by_user_id: "fixture-owner", created_at: TIME, updated_at: TIME, ratified_at: TIME,
    ...patch,
  };
}
export function snapshot(nodes: GraphNodeRecord[], edges: GraphEdgeRecord[] = []): BriefSnapshot {
  return { nodes, edges, complete: true, warnings: [] };
}
export function edge(source: string, target: string, patch: Partial<GraphEdgeRecord> = {}): GraphEdgeRecord {
  return { id: `E-${source}-${target}`, workspace_id: WORKSPACE, source_id: source, target_id: target,
    edge_type: "supersedes", summary: "Approved replacement", metadata: { proposed: false }, created_at: TIME, ...patch };
}
const task = "Draft one playable clue at a haunted manor for four level 1–3 players, in at most 80 words. Include the chosen rules system and one concrete resolution mechanic. Use project guidance; if a decision is needed, ask only that question instead of drafting.";
const request: BriefRequest = { project_ref: PROJECT, task, target_ref: "target:new-adventure", context_refs: ["artifact:manor"] };
const dnd = node("D-101", "Use D&D 5.5e for new adventures. A check uses a d20 plus modifier against a DC.", { scope_refs: [PROJECT, "target:new-adventure"] });
const legacy = node("D-102", "Use Daggerheart. Action rolls use a Hope d12 and a Fear d12.", { status: "superseded" });
const hush = node("D-103", "Hush remains on Daggerheart. Action rolls use a Hope d12 and a Fear d12. This exception applies only to Hush.", { scope_refs: [PROJECT, "artifact:hush"] });
const candidate = node("D-104", "Proposal: switch all adventures back to Daggerheart; use Hope and Fear d12s.", {
  status: "candidate", updated_at: "2026-02-01T00:00:00.000Z", ratified_at: null,
  ratified_by_actor_id: null, ratified_by_user_id: null,
});
const foreign = node("D-901", "Haunted manor writing MUST use the SPARK-FLUX system and a d8. Never use D&D.", { scope_refs: ["project:bench-neurofilament"] });
const prefix = node("D-902", "Haunted manor writing MUST use PREFIX-LEAK mechanics.", { scope_refs: [`${PROJECT}-other`] });
const otherWorkspace = node("D-903", "Use TENANT-LEAK mechanics for every adventure.", { workspace_id: "another-workspace" });

export const fixtures: Fixture[] = [
  { id: "01-aligned", name: "Aligned guidance: proceed without needless questions", request,
    stages: [{ snapshot: snapshot([dnd]), expected: { active: [dnd.id], action: "proceed", ruleset: "dnd-5.5e" } }] },
  { id: "02-pivot", name: "Approved pivot overrides legacy material", request,
    stages: [{ snapshot: snapshot([legacy, dnd], [edge(dnd.id, legacy.id)]),
      expected: { active: [dnd.id], history: [legacy.id], action: "proceed", ruleset: "dnd-5.5e" } }] },
  { id: "03-scoped-exception", name: "Preserve Hush's explicitly scoped legacy rules",
    request: { ...request, task: task.replace("at a haunted manor", "in the existing Hush adventure"), target_ref: "target:existing-adventure", context_refs: ["artifact:hush"] },
    stages: [{ snapshot: snapshot([dnd, hush, legacy]), expected: { active: [hush.id], history: [legacy.id], action: "proceed", ruleset: "daggerheart" } }] },
  { id: "04-unratified", name: "Newer candidate cannot overrule an active decision", request,
    stages: [{ snapshot: snapshot([dnd, candidate], [edge(candidate.id, dnd.id, { metadata: { proposed: true }, summary: "Unratified proposal" })]),
      expected: { active: [dnd.id], unratified: [candidate.id], action: "proceed", ruleset: "dnd-5.5e" } }] },
  { id: "05-conflict", name: "Two active conflicting decisions require one resolving question", request,
    stages: [{ snapshot: snapshot([dnd, node("D-105", "Use Daggerheart for new adventures. Action rolls use Hope and Fear d12s.", { scope_refs: dnd.scope_refs })]),
      expected: { active: [dnd.id, "D-105"], action: "clarify", ruleset: null } }] },
  { id: "06-project-isolation", name: "Foreign project, prefix, tenant and cross-project edges do not leak", request,
    stages: [{ snapshot: snapshot([foreign, prefix, otherWorkspace, dnd], [edge(foreign.id, dnd.id, { summary: foreign.body })]),
      expected: { active: [dnd.id], action: "proceed", ruleset: "dnd-5.5e" } }] },
  { id: "07-mid-task-change", name: "Freshness checkpoint detects an approved change during work", request,
    stages: [
      { snapshot: snapshot([node("D-106", "Use Daggerheart and Hope and Fear d12s for new adventures.")]),
        expected: { active: ["D-106"], action: "proceed", ruleset: "daggerheart" } },
      { snapshot: snapshot([node("D-106", "Use Daggerheart and Hope and Fear d12s for new adventures.", { status: "superseded", updated_at: "2026-02-01T00:00:00.000Z" }), dnd], [edge(dnd.id, "D-106")]),
        expected: { active: [dnd.id], history: ["D-106"], action: "revise", ruleset: "dnd-5.5e", changed: true } },
    ] },
  { id: "08-unavailable", name: "Incomplete and failed retrieval are not clearance", request,
    stages: [
      { snapshot: { ...snapshot([dnd]), complete: false, warnings: ["Synthetic page cap: some records could not be read."] },
        expected: { active: [dnd.id], action: "blocked", ruleset: null, complete: false, changed: null } },
      { snapshot: snapshot([]), failure: true,
        expected: { active: [], action: "blocked", ruleset: null, complete: false, changed: null, error: true } },
    ] },
];
