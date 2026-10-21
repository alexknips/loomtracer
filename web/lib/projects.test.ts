import { describe, expect, it } from "vitest";
import { demoGraph, demoProjects } from "@/lib/demo";
import { ancestors, inheritedNodes, moveImpact, projectGraph, selectProjectNodes } from "./projects";
import { buildTaskBrief } from "./server/task-brief";
import type { GraphNodeRecord } from "@/types/graph";

const [neuro, world, echeasea, hush, manor] = demoProjects;
const node = (id: string, patch: Partial<GraphNodeRecord> = {}): GraphNodeRecord => ({
  ...demoGraph.nodes[0], id, type: "Decision", status: "active", scope_refs: [],
  project_id: echeasea.id, inheritance_mode: "local", ...patch,
});

describe("project ownership and inheritance", () => {
  it("inherits only explicitly shared active decisions from ancestors, preserving source and restrictions", () => {
    const nodes = [node("shared", { inheritance_mode: "descendants", scope_refs: ["workflow:adventure-writing"] }),
      node("local"), node("candidate", { inheritance_mode: "descendants", status: "candidate" }),
      node("retired", { inheritance_mode: "descendants", status: "superseded" }),
      node("sibling", { project_id: manor.id, inheritance_mode: "descendants" }),
      node("foreign", { workspace_id: "elsewhere", inheritance_mode: "descendants" })];
    const inherited = inheritedNodes(nodes, demoProjects, hush.id);
    expect(inherited.map(n => n.id)).toEqual(["shared"]);
    expect(inherited[0].inherited_from_project_id).toBe(echeasea.id);
    expect(inherited[0].scope_refs).toEqual(["workflow:adventure-writing"]);
    expect(nodes[0].inherited_from_project_id).toBeUndefined();
  });

  it("uses ownership instead of old project tags and keeps unassigned records visible", () => {
    const nodes = [node("wrong-owner", { project_id: world.id, scope_refs: [hush.ref, echeasea.ref] }),
      node("own", { project_id: hush.id }), node("unassigned", { project_id: null, scope_refs: [hush.ref] })];
    expect(selectProjectNodes(nodes, demoProjects, hush.ref).map(n => n.id)).toEqual(["own"]);
    expect(selectProjectNodes(nodes, demoProjects, "unassigned").map(n => n.id)).toEqual(["unassigned"]);
    expect(() => selectProjectNodes(nodes, demoProjects, "project:invented")).toThrow("Project not found");
  });

  it("rejects cycles and missing or foreign parents", () => {
    expect(() => moveImpact(demoProjects, [], echeasea.id, hush.id)).toThrow("cycle");
    expect(() => ancestors(demoProjects.map(p => p.id === hush.id ? { ...p, parent_id: "missing" } : p), hush.id)).toThrow("unavailable");
    expect(() => ancestors(demoProjects.map(p => p.id === echeasea.id ? { ...p, workspace_id: "foreign" } : p), hush.id)).toThrow("unavailable");
  });

  it("previews inherited guidance gained and lost when moving a subtree", () => {
    const nodes = [node("old", { inheritance_mode: "descendants" }), node("new", { project_id: world.id, inheritance_mode: "descendants" })];
    const result = moveImpact(demoProjects, nodes, hush.id, world.id);
    expect(result[0].lost.map(n => n.id)).toEqual(["old"]);
    expect(result[0].gained.map(n => n.id)).toEqual(["new"]);
    expect(result).toHaveLength(1);
  });

  it("neighborhoods cannot traverse an external project as a shortcut", () => {
    const a = node("a", { project_id: hush.id }); const b = node("b", { project_id: hush.id });
    const other = node("other", { project_id: world.id });
    const edge = (id: string, source_id: string, target_id: string) => ({ id, source_id, target_id,
      workspace_id: hush.workspace_id, edge_type: "supports", summary: "", metadata: {}, created_at: "now" });
    const payload = projectGraph({ ...demoGraph, nodes: [a, b, other], edges: [edge("e1", "a", "other"), edge("e2", "other", "b")] }, hush.ref, { root: "a", view: "neighborhood", depth: 2 });
    expect(payload.nodes.map(n => n.id)).toEqual(["a"]);
    expect(payload.edges).toEqual([]);
    expect(payload.cross_project_edges).toHaveLength(2);
  });

  it("shows local nodes by default and never silently substitutes an inherited node for local ownership", () => {
    const payload = projectGraph(demoGraph, hush.ref);
    expect(payload.nodes.map(n => n.id)).toEqual(["D-demo-hush"]);
    expect(payload.inherited_nodes?.map(n => n.id)).toEqual(["D-demo-setting"]);
    const included = projectGraph(demoGraph, hush.ref, { includeInherited: true });
    expect(included.nodes).toHaveLength(2);
    expect(projectGraph(demoGraph, world.ref).nodes).toHaveLength(0);
  });

  it("invalidates a child's brief when ancestor guidance or hierarchy changes, not for an unrelated project", () => {
    const nodes = [node("shared", { inheritance_mode: "descendants" }), node("other", { project_id: neuro.id })];
    const snapshot = { nodes, edges: [], projects: demoProjects, complete: true, warnings: [] };
    const request = { project_ref: hush.ref, task: "Write" };
    const first = buildTaskBrief(snapshot, hush.workspace_id, request, "now");
    const changed = buildTaskBrief({ ...snapshot, nodes: [{ ...nodes[0], body: "New guidance" }, nodes[1]] }, hush.workspace_id,
      { ...request, previous_fingerprint: first.fingerprint }, "later");
    expect(changed.changed).toBe(true);
    const unrelated = buildTaskBrief({ ...snapshot, nodes: [nodes[0], { ...nodes[1], body: "Other change" }] }, hush.workspace_id, request, "later");
    expect(unrelated.fingerprint).toBe(first.fingerprint);
    const moved = buildTaskBrief({ ...snapshot, projects: demoProjects.map(p => p.id === hush.id ? { ...p, parent_id: world.id } : p) }, hush.workspace_id, request, "later");
    expect(moved.fingerprint).not.toBe(first.fingerprint);
    expect(moved.groups.active_decisions).toHaveLength(0);
  });

  it("does not erase inherited workflow restrictions or silently let local decisions override parents", () => {
    const nodes = [node("parent", { inheritance_mode: "descendants", scope_refs: ["workflow:adventure-writing"] }), node("child", { project_id: hush.id })];
    const snapshot = { nodes, edges: [], projects: demoProjects, complete: true, warnings: [] };
    const request = { project_ref: hush.ref, task: "Write" };
    const ambiguous = buildTaskBrief(snapshot, hush.workspace_id, request, "now");
    expect(ambiguous.groups.active_decisions.map(n => n.id)).toEqual(["child"]);
    expect(ambiguous.scope_questions[0].node_id).toBe("parent");
    const resolved = buildTaskBrief(snapshot, hush.workspace_id, { ...request, workflow_ref: "workflow:adventure-writing" }, "now");
    expect(resolved.groups.active_decisions.map(n => n.id)).toEqual(["child", "parent"]);
  });
});
