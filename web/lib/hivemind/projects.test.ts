import { describe, expect, it } from "vitest";
import { listProjects, projectPath, stored, UNASSIGNED } from "./projects";
import type { GraphDecision, GraphEdge, GraphResponse } from "./types";
import fixture from "./fixtures/graph.json";

const graph = fixture as unknown as GraphResponse;
const decision = (id: string, project: string | null, status: GraphDecision["status"] = "accepted"): GraphDecision =>
  ({ id, title: id, rationale: null, status, project, topic_keys: [], decided_at: null, event_origin: null, deciders: [], slug: null });
const partOf = (child: string, parent: string, reversed = false): GraphEdge => reversed
  ? { from: `Project:${parent}`, to: `Project:${child}`, relation: "PART_OF", label: "contains", reversed }
  : { from: `Project:${child}`, to: `Project:${parent}`, relation: "PART_OF", label: "is part of", reversed };

describe("projects of a graph", () => {
  it("lists registered projects with their place in the hierarchy and their decisions by status", () => {
    const projects = listProjects(graph);
    expect(projects.map(project => project.ref)).toEqual(["pricing", "pricing-experiments"]);
    const [parent, child] = projects;
    expect(parent).toMatchObject({ name: "Pricing study", parent: null, children: ["pricing-experiments"], counts: { total: 3, accepted: 1, contested: 1, superseded: 1, proposed: 0, rejected: 0 } });
    expect(child).toMatchObject({ name: "Pricing experiments", parent: "pricing", counts: { total: 1, proposed: 1 } });
    expect(projectPath(projects, "pricing-experiments")).toBe("Pricing study / Pricing experiments");
  });

  it("reads a part_of arrow in the direction the graph stores it, whichever way the arrow runs", () => {
    expect(stored(partOf("a", "b"))).toEqual({ source: "Project:a", target: "Project:b" });
    expect(stored(partOf("a", "b", true))).toEqual({ source: "Project:a", target: "Project:b" });
    const base: GraphResponse = { decisions: [], edges: [partOf("a", "b", true)], nodes: [{ id: "Project:a", kind: "Project" }, { id: "Project:b", kind: "Project" }] };
    expect(listProjects(base).find(project => project.ref === "a")?.parent).toBe("b");
  });

  it("lists personal and unassigned decisions as projects of their own", () => {
    const base: GraphResponse = { nodes: [], edges: [], decisions: [decision("d1", "personal:agent:claude"), decision("d2", null, "proposed")] };
    expect(listProjects(base).map(project => [project.ref, project.name, project.counts.total])).toEqual([["personal:agent:claude", "personal:agent:claude", 1], [UNASSIGNED, "Unassigned", 1]]);
  });

  it("survives a part_of cycle without hiding or looping", () => {
    const base: GraphResponse = { decisions: [], edges: [partOf("a", "b"), partOf("b", "a")], nodes: [{ id: "Project:a", kind: "Project", label: "A" }, { id: "Project:b", kind: "Project", label: "B" }] };
    const projects = listProjects(base);
    expect(projects.map(project => project.ref).sort()).toEqual(["a", "b"]);
    expect(projects.filter(project => project.parent === null).length).toBeGreaterThanOrEqual(1);
    expect(() => projectPath(projects, "a")).not.toThrow();
  });
});
