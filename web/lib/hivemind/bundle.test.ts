import { describe, expect, it } from "vitest";
import { entryText, getView, relatedConnections, validateBundle } from "@reasoning";
import { buildProjectBundle, LEDGER_ACTOR_ID } from "./bundle";
import type { GraphResponse } from "./types";
import fixture from "./fixtures/graph.json";

const graph = fixture as unknown as GraphResponse;
const NOW = "2026-10-21T03:00:00.000Z";
const build = (ref: string) => buildProjectBundle(graph, ref, "local", NOW)!;
const decision = (title: string) => graph.decisions.find(item => item.title === title)!;

describe("a project's graph as a reasoning bundle", () => {
  it.each(["pricing", "pricing-experiments"])("passes the bundle validation for %s", ref => {
    const result = validateBundle(build(ref).bundle);
    expect(result.errors).toEqual([]);
    expect(result.ok).toBe(true);
  });

  it("is a pure function of the graph", () => {
    expect(build("pricing")).toEqual(build("pricing"));
  });

  it("answers null for a project the graph does not hold", () => {
    expect(buildProjectBundle(graph, "no-such-project", "local", NOW)).toBeNull();
  });

  it("owns the bundle by tenant and project", () => {
    const { bundle } = build("pricing");
    expect(bundle.namespace).toEqual({ datasetId: "hivemind:local:pricing", workspaceId: "local", projectId: "pricing" });
    expect(bundle.title).toBe("Pricing study");
    expect(bundle.contexts.map(context => context.kind)).toEqual(["ledger_record"]);
  });

  it("makes each decision an entry whose text is its title and rationale, with its status as a label", () => {
    const { bundle } = build("pricing");
    const view = getView(bundle);
    const contested = decision("Show the price before the invite is sent");
    const entry = view.entries.find(item => item.id === contested.id)!;
    expect(entry.labels).toEqual(["Decision", "status:contested"]);
    expect(entryText(bundle, entry)).toBe(`${contested.title}\n\n${contested.rationale}`);
    expect(view.entries.find(item => item.id === decision("Price per seat").id)!.labels).toContain("status:superseded");
    expect(view.entries.find(item => item.id === decision("Test a team plan with included usage").id)!.labels).toContain("status:accepted");
  });

  it("keeps what was weighed: each option is an entry, the chosen one marked", () => {
    const { bundle } = build("pricing");
    const options = bundle.entries.filter(entry => entry.labels[0] === "Option");
    expect(options.map(entry => entryText(bundle, entry)).sort()).toEqual(["Price after", "Price first", "Pure per seat", "Pure usage", "Per seat", "Team plan with allowance", "Usage based"].sort());
    expect(options.filter(entry => entry.labels.includes("chosen")).map(entry => entryText(bundle, entry)).sort()).toEqual(["Per seat", "Price first", "Team plan with allowance"].sort());
  });

  it("draws each edge once, in the direction the graph stores it", () => {
    const { bundle } = build("pricing");
    const ids = bundle.connections.map(connection => connection.id);
    expect(new Set(ids).size).toBe(ids.length);
    const supersedes = bundle.connections.filter(connection => connection.relation === "legacy:supersedes");
    expect(supersedes).toHaveLength(1);
    // The newer decision makes the claim, the older one is what it replaces.
    expect(supersedes[0]!.participants).toEqual([
      { entryId: decision("Test a team plan with included usage").id, role: "source" },
      { entryId: decision("Price per seat").id, role: "target" },
    ]);
  });

  it("shows what a decision rests on", () => {
    const { bundle } = build("pricing");
    const assumption = bundle.entries.find(entry => entryText(bundle, entry).startsWith("Teams prefer a bill"))!;
    const rests = relatedConnections(getView(bundle), assumption.id);
    expect(rests.map(connection => connection.relation)).toContain("legacy:premised_on");
  });

  it("keeps an edge into another project as a pointer, without that project's text", () => {
    const { bundle } = build("pricing-experiments");
    const parent = decision("Test a team plan with included usage");
    expect(bundle.externalEntryRefs).toEqual([{ id: `external:${parent.id}`, entryId: parent.id, originalId: parent.id, namespace: { datasetId: "hivemind:local:pricing", workspaceId: "local", projectId: "pricing" } }]);
    expect(bundle.entries.some(entry => entry.id === parent.id)).toBe(false);
    expect(JSON.stringify(bundle.sources)).not.toContain(parent.rationale!);
    const link = bundle.connections.find(connection => connection.relation === "legacy:follows_from")!;
    expect(link.participants.map(item => item.entryId)).toContain(`external:${parent.id}`);
    expect(getView(bundle).externalConnections?.map(connection => connection.id)).toContain(link.id);
  });

  it("shows a refuting piece of evidence beside the assumption it refutes", () => {
    const { bundle } = build("pricing-experiments");
    const refutes = bundle.connections.find(connection => connection.relation === "legacy:refutes")!;
    const [evidence, assumption] = refutes.participants.map(item => bundle.entries.find(entry => entry.id === item.entryId)!);
    expect(evidence!.labels).toEqual(["Evidence"]);
    expect(entryText(bundle, assumption!)).toBe("Invite behaviour is a fair proxy for willingness to collaborate");
  });

  it("replays the record in ledger order as the cutoff moves", () => {
    const { bundle } = build("pricing");
    const counts = [0, 1, 4, bundle.sources.length].map(cutoff => getView(bundle, undefined, cutoff).entries.length);
    expect(counts[0]).toBe(0);
    expect(counts).toEqual([...counts].sort((a, b) => a - b));
    expect(counts.at(-1)).toBe(bundle.entries.length);
    // The first record is the oldest decision of the project.
    expect(bundle.sources[0]!.originalId).toBe(decision("Price per seat").id);
    // Moving the cutoff never shows a connection before both its ends.
    for (let cutoff = 0; cutoff <= bundle.sources.length; cutoff++) {
      const view = getView(bundle, undefined, cutoff);
      const visible = new Set(view.entries.map(entry => entry.id));
      expect(view.connections.every(connection => connection.participants.every(item => visible.has(item.entryId)))).toBe(true);
    }
  });

  it("names each record's source in the ledger", () => {
    const { bundle } = build("pricing");
    const first = bundle.sources[0]!;
    expect(first.sourceRef).toBe(`hivemind://local/${decision("Price per seat").id}#event-${decision("Price per seat").event_origin}`);
    expect(first.role).toBe("record");
    expect(bundle.manifest.segments).toHaveLength(bundle.sources.length);
  });

  it("attributes records to the actors who recorded them, never to the ledger itself", () => {
    const { bundle } = build("pricing");
    const teamPlan = decision("Test a team plan with included usage");
    expect(bundle.entries.find(entry => entry.id === teamPlan.id)!.provenance).toMatchObject({ expressedBy: "agent:claude:analyst", recordedBy: "agent:claude:analyst", occurredAt: teamPlan.decided_at, basis: "direct" });
    expect(bundle.entries.every(entry => entry.provenance.expressedBy !== LEDGER_ACTOR_ID)).toBe(true);
    expect(bundle.actors.find(actor => actor.id === "human:alice")?.kind).toBe("human");
    expect(bundle.actors.find(actor => actor.id === "agent:claude:analyst")?.kind).toBe("agent");
  });

  it("is a valid, empty bundle for a registered project with no decisions", () => {
    const empty: GraphResponse = { ...graph, nodes: [...graph.nodes, { id: "Project:empty-project", kind: "Project", label: "Empty" }] };
    const { bundle } = buildProjectBundle(empty, "empty-project", "local", NOW)!;
    expect(bundle.entries).toEqual([]);
    expect(bundle.runs).toEqual([]);
    expect(validateBundle(bundle).ok).toBe(true);
  });
});
