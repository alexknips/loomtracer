import { describe, expect, it, vi } from "vitest";
import { createHiveMindClient } from "./client";
import { decisionDetail, decisionIdFromSegment, readDecision } from "./decision";
import type { DecisionBrief, GraphResponse, Timeline } from "./types";
import graphFixture from "./fixtures/graph.json";
import detailFixture from "./fixtures/decisions.json";

// Captured from a scratch `hivemind serve`: the whole graph, and for each decision GET /v1/decisions/{id}, its
// timeline and GET /v1/graph?decision={id}, the graph the decision page reads.
const graph = graphFixture as unknown as GraphResponse;
const details = detailFixture as unknown as Record<string, { brief: DecisionBrief; timeline: Timeline; graph: GraphResponse }>;
const detail = (title: string) => decisionDetail(details[title]!.brief, details[title]!.timeline, details[title]!.graph);

describe("one decision, read for a person", () => {
  it("tells a superseded decision as itself, and names what replaced it", () => {
    const old = detail("Price per seat");
    expect(old).toMatchObject({ title: "Price per seat", status: "superseded", proposedBy: "human:alice" });
    expect(old.supersededBy.map(other => other.title)).toEqual(["Test a team plan with included usage"]);
    expect(old.supersedes).toEqual([]);
    expect(old.timeline.map(entry => entry.kind)).toEqual(["recorded", "accepted", "superseded"]);
    expect(old.options).toEqual(expect.arrayContaining([{ id: expect.any(String), label: "Per seat", chosen: true }, { id: expect.any(String), label: "Usage based", chosen: false }]));
    expect(old.assumptions.map(item => item.statement)).toEqual(["Teams prefer a bill they can predict over one that follows activity"]);
  });

  it("tells the replacement as itself, with what it was based on and what it replaced", () => {
    const current = detail("Test a team plan with included usage");
    expect(current.status).toBe("accepted");
    expect(current.supersedes.map(other => other.title)).toEqual(["Price per seat"]);
    expect(current.evidence.map(item => item.text)).toEqual(["Seat study: occasional collaborators were invited less often when each seat cost extra"]);
    expect(current.binds?.statement).toBe("The next pricing experiment");
    expect(current.undoCost?.statement).toBe("One experiment cycle");
  });

  it("keeps both sides of a disagreement", () => {
    const contested = detail("Show the price before the invite is sent");
    expect(contested.status).toBe("contested");
    expect(contested.contest).toEqual({ acceptedBy: ["human:bob"], rejectedBy: ["human:carol"] });
    expect(contested.timeline.at(-1)).toMatchObject({ kind: "rejected", actor_id: "human:carol", reason: expect.stringContaining("suppress invites") });
  });

  it("names a decision a proposal rests on, from another project, and has no contest", () => {
    const proposed = detail("Run the allowance experiment on the invite flow");
    expect(proposed).toMatchObject({ status: "proposed", contest: null, deciders: [] });
    expect(proposed.premises).toEqual([{ id: expect.any(String), title: "Test a team plan with included usage", status: "accepted" }]);
    expect(proposed.project).toEqual({ ref: "pricing-experiments", label: "Pricing experiments" });
  });
});

describe("a decision's own graph", () => {
  it("is all the page needs: every decision reads the same from it as from the whole tenant graph", () => {
    const titles = Object.keys(details);
    expect(titles.length).toBeGreaterThanOrEqual(4);
    for (const title of titles) {
      const { brief, timeline, graph: own } = details[title]!;
      expect(decisionDetail(brief, timeline, own), title).toEqual(decisionDetail(brief, timeline, graph));
      expect(own.nodes.length, title).toBeLessThan(graph.nodes.length);
    }
  });

  it("holds each edge once, with both ends in it", () => {
    for (const [title, { graph: own }] of Object.entries(details)) {
      const nodes = new Set(own.nodes.map(node => node.id));
      const edges = own.edges.map(edge => `${edge.relation} ${edge.from} ${edge.to}`);
      expect(new Set(edges).size, title).toBe(edges.length);
      expect(own.edges.filter(edge => !nodes.has(edge.from) || !nodes.has(edge.to)), title).toEqual([]);
    }
  });
});

describe("the decision id in a decision page's path", () => {
  it.each(["capture:105186:3", "decision-01dcec47-2b6c-7f2e-8a5d-5b2f0c9e4a11", "capture:7:0"])("reads %s back out of the link a page wrote for it", id => {
    expect(decisionIdFromSegment(encodeURIComponent(id))).toBe(id);
  });

  it("names no decision when the segment does not decode", () => {
    expect(decisionIdFromSegment("capture%3A%E0%A4%A")).toBeNull();
  });

  it("asks the server for a classifier draft once-escaped, as the link to it was written", async () => {
    const fetchImpl = vi.fn(async () => new Response(JSON.stringify({ data: {} }), { status: 200, headers: { "content-type": "application/json" } }));
    const client = createHiveMindClient({ HIVEMIND_API_URL: "http://127.0.0.1:18736" }, fetchImpl as unknown as typeof fetch);
    await client.decision(decisionIdFromSegment("capture%3A105186%3A3")!);
    await client.timeline(decisionIdFromSegment("capture%3A105186%3A3")!);
    expect(fetchImpl.mock.calls.map(call => String((call as unknown as [URL])[0]))).toEqual([
      "http://127.0.0.1:18736/v1/decisions/capture%3A105186%3A3",
      "http://127.0.0.1:18736/v1/decisions/capture%3A105186%3A3/timeline",
    ]);
  });
});

describe("a decision page's reads", () => {
  it("reads the decision, its timeline and the graph around it, and never the whole tenant graph", async () => {
    const { brief, timeline } = details["Price per seat"]!;
    const client = { decision: vi.fn(async () => brief), timeline: vi.fn(async () => timeline), decisionGraph: vi.fn(async () => graph), graph: vi.fn(async () => graph) };
    const read = await readDecision(client, brief.id);
    expect(read).toEqual(decisionDetail(brief, timeline, graph));
    expect(client.decision.mock.calls).toEqual([[brief.id]]);
    expect(client.timeline.mock.calls).toEqual([[brief.id]]);
    expect(client.decisionGraph.mock.calls).toEqual([[brief.id]]);
    expect(client.graph).not.toHaveBeenCalled();
  });
});
