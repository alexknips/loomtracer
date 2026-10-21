import { describe, expect, it } from "vitest";
import { filterPublicDemo, publicDemoEdges, publicDemoNodes } from "./public-demo";

describe("fictional public pricing study", () => {
  it("marks every source as illustrative and never represents its proposed test as approved", () => {
    const decisions = publicDemoNodes.filter(node => node.kind === "Decision");
    expect(decisions).toHaveLength(1);
    expect(decisions[0].status).toBe("Candidate");
    expect(decisions[0].reconsider.length).toBeGreaterThan(0);
    expect(publicDemoNodes.filter(node => node.kind !== "Decision").every(node => node.status === "Observed")).toBe(true);
    expect(publicDemoNodes.every(node => /illustrative/i.test(node.source) && node.detail && node.stage)).toBe(true);
    const narrative = publicDemoNodes.map(node => `${node.summary} ${node.detail} ${node.source} ${node.stage}`).join(" ");
    expect(narrative).not.toMatch(/\b20\d{2}\b|\d+%|\$\d/);
  });

  it("contains no private identifiers or workspace data", () => {
    const payload = JSON.stringify({ nodes: publicDemoNodes, edges: publicDemoEdges });
    expect(payload).not.toMatch(/workspace_id|actor_id|source_threads|project:|[a-f0-9]{8}-[a-f0-9]{4}-|@|\/Users\/|nf_pat_/);
  });

  it("filters each investigation independently and combines search with record type", () => {
    expect(filterPublicDemo("", "Attempt").map(node => node.id)).toEqual(["seat-study", "usage-study"]);
    expect(filterPublicDemo("  FORECAST  ", "all").map(node => node.id)).toEqual(["seat-study"]);
    expect(filterPublicDemo("uncertain", "Attempt").map(node => node.id)).toEqual(["usage-study"]);
    expect(filterPublicDemo("forecast", "Evidence")).toEqual([]);
    expect(filterPublicDemo("nothing-matches-this", "all")).toEqual([]);
  });

  it("connects both investigations through shared evidence to the candidate", () => {
    const ids = new Set(publicDemoNodes.map(node => node.id));
    expect(ids.size).toBe(publicDemoNodes.length);
    expect(new Set(publicDemoEdges.map(edge => edge.id)).size).toBe(publicDemoEdges.length);
    expect(publicDemoEdges.every(edge => ids.has(edge.source) && ids.has(edge.target))).toBe(true);
    const evidence = publicDemoNodes.find(node => node.kind === "Evidence")!;
    const candidate = publicDemoNodes.find(node => node.kind === "Decision")!;
    const studies = publicDemoNodes.filter(node => node.kind === "Attempt");
    for (const study of studies) {
      expect(publicDemoEdges).toContainEqual(expect.objectContaining({ source: study.id, target: evidence.id, label: "informs" }));
    }
    expect(publicDemoEdges).toContainEqual(expect.objectContaining({ source: evidence.id, target: candidate.id, label: "supports" }));
    expect(publicDemoEdges.some(edge => edge.source === candidate.id)).toBe(false);
  });
});
