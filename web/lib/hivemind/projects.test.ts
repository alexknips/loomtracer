import { describe, expect, it } from "vitest";
import { projectPath, stored, summarizeProjects } from "./projects";
import type { DecisionCounts, GraphEdge, ProjectEntry } from "./types";
import fixture from "./fixtures/projects.json";

const entries = (fixture as unknown as { data: { items: ProjectEntry[] } }).data.items;
const counts = (total: number, by: Partial<DecisionCounts> = {}): DecisionCounts => ({ total, proposed: 0, accepted: 0, contested: 0, rejected: 0, superseded: 0, ...by });
const entry = (handle: string, parent: string | null, name = handle, decisions = counts(0)): ProjectEntry => ({ handle, name, personal: false, parent, decisions });
const partOf = (child: string, parent: string, reversed = false): GraphEdge => reversed
  ? { from: `Project:${parent}`, to: `Project:${child}`, relation: "PART_OF", label: "contains", reversed }
  : { from: `Project:${child}`, to: `Project:${parent}`, relation: "PART_OF", label: "is part of", reversed };

describe("projects of GET /v1/projects", () => {
  it("lists registered projects with their place in the hierarchy and their decisions by status", () => {
    const projects = summarizeProjects(entries);
    expect(projects.map(project => project.ref)).toEqual(["pricing", "pricing-experiments"]);
    const [parent, child] = projects;
    expect(parent).toMatchObject({ name: "Pricing study", parent: null, children: ["pricing-experiments"], counts: { total: 3, accepted: 1, contested: 1, superseded: 1, proposed: 0, rejected: 0 } });
    expect(child).toMatchObject({ name: "Pricing experiments", parent: "pricing", counts: { total: 1, proposed: 1 } });
    expect(projectPath(projects, "pricing-experiments")).toBe("Pricing study / Pricing experiments");
  });

  it("reads a part_of arrow in the direction the graph stores it, whichever way the arrow runs", () => {
    expect(stored(partOf("a", "b"))).toEqual({ source: "Project:a", target: "Project:b" });
    expect(stored(partOf("a", "b", true))).toEqual({ source: "Project:a", target: "Project:b" });
  });

  it("lists a personal project beside the registered ones, under its own name", () => {
    const personal: ProjectEntry = { handle: "personal:agent:claude", name: "agent:claude", personal: true, parent: null, decisions: counts(2, { accepted: 1, proposed: 1 }) };
    const projects = summarizeProjects([entry("pricing", null, "Pricing study"), personal]);
    expect(projects.map(project => [project.ref, project.name, project.counts.total])).toEqual([["personal:agent:claude", "agent:claude", 2], ["pricing", "Pricing study", 0]]);
  });

  it("names a project by its handle when the server sent no name", () => {
    expect(summarizeProjects([entry("pricing", null, "")]).map(project => project.name)).toEqual(["pricing"]);
  });

  it("keeps a project whose parent the page does not hold as a root", () => {
    const projects = summarizeProjects([entry("child", "missing-parent")]);
    expect(projects.map(project => [project.ref, project.parent])).toEqual([["child", null]]);
  });

  it("survives a part_of cycle without hiding or looping", () => {
    const projects = summarizeProjects([entry("a", "b", "A"), entry("b", "a", "B")]);
    expect(projects.map(project => project.ref).sort()).toEqual(["a", "b"]);
    expect(projects.filter(project => project.parent === null).length).toBeGreaterThanOrEqual(1);
    expect(() => projectPath(projects, "a")).not.toThrow();
  });
});
