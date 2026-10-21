import { describe, expect, it } from "vitest";
import { decisionDetail } from "./decision";
import type { DecisionBrief, GraphResponse, Timeline } from "./types";
import graphFixture from "./fixtures/graph.json";
import detailFixture from "./fixtures/decisions.json";

// Captured from a scratch `hivemind serve`: the graph, and for each decision GET /v1/decisions/{id} and its timeline.
const graph = graphFixture as unknown as GraphResponse;
const details = detailFixture as unknown as Record<string, { brief: DecisionBrief; timeline: Timeline }>;
const detail = (title: string) => decisionDetail(details[title]!.brief, details[title]!.timeline, graph);

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
