/**
 * Renders one project of the Loomtracer decision graph as a reasoning bundle, the shape the history
 * views (timeline, trace map, sources, inspector) read. A pure function of `GET /v1/graph?project=`
 * (the project's newest decisions and what hangs off them) and that project's entry in the project list:
 *
 * - each Decision, Option, Evidence, Hypothesis and Question node is one record (a source with the
 *   node's words) and one entry whose span covers those words;
 * - each graph edge between two such nodes is one connection, kept in the direction the graph
 *   stores it (`legacy:<relation>`; the UI reads the word from lib/loomtracer/relations.ts);
 * - an edge to a decision filed under another project keeps the edge and points at an external
 *   reference, so the other project's text is not copied here;
 * - sources are ordered by the decision that introduced them, in ledger order, so moving the
 *   cutoff replays the project's record in the order it was made.
 *
 * It states only what the graph holds. Status and deciders come from the graph's derived fields. A page
 * that stopped short of the project's older decisions says so in the run's uncertainties.
 * Requests, blockers, notifications, reaffirmations and project links are not drawn here.
 */
import { createHash } from "node:crypto";
import type { Actor, Connection, Entry, ExternalEntryRef, ReasoningBundle, SourceMessage } from "@reasoning";
import { nodeKind, nodeRawId, projectOf, stored } from "./projects";
import type { GraphDecision, GraphNode, GraphResponse, ProjectSummary } from "./types";

export const LEDGER_ACTOR_ID = "loomtracer:ledger";
export type ProjectBundle = { bundle: ReasoningBundle; head: string };

/** Node kinds that become records. Everything else (actors, projects, requests, blockers...) does not. */
const RECORD_KINDS = new Set(["Decision", "Option", "Evidence", "Hypothesis", "Question"]);
const LABEL: Record<string, string> = { Decision: "Decision", Option: "Option", Evidence: "Evidence", Hypothesis: "Hypothesis", Question: "Question" };
const PROPOSER_EDGES = new Set(["PROPOSED_BY"]);
const sha256 = (text: string) => createHash("sha256").update(text).digest("hex");
const actorKind = (id: string): Actor["kind"] => id.startsWith("human:") ? "human" : id.startsWith("agent:") ? "agent" : id.startsWith("service:") || id === LEDGER_ACTOR_ID ? "system" : "unknown";
export const bundleId = (tenant: string, projectRef: string) => `loomtracer:${tenant}:${projectRef}`;

type Member = { node: GraphNode; raw: string; introducedBy: GraphDecision; order: number };

export function buildProjectBundle(graph: GraphResponse, project: Pick<ProjectSummary, "ref" | "name">, tenant: string, now: string): ProjectBundle {
  const projectRef = project.ref;
  const nodes = new Map(graph.nodes.map(node => [node.id, node]));
  const decisions = new Map(graph.decisions.map(decision => [decision.id, decision]));
  const edges = graph.edges.filter(edge => !edge.derived).map(edge => ({ ...stored(edge), relation: edge.relation, edge }));
  const outgoing = new Map<string, typeof edges>(), incoming = new Map<string, typeof edges>();
  for (const edge of edges) {
    (outgoing.get(edge.source) ?? outgoing.set(edge.source, []).get(edge.source)!).push(edge);
    (incoming.get(edge.target) ?? incoming.set(edge.target, []).get(edge.target)!).push(edge);
  }
  const targets = (id: string, ...relations: string[]) => (outgoing.get(id) ?? []).filter(edge => relations.includes(edge.relation)).map(edge => edge.target);

  // The project's decisions in ledger order; each pulls in what it weighs, rests on and answers.
  const own = graph.decisions.filter(decision => projectOf(decision) === projectRef)
    .sort((a, b) => (a.event_origin ?? Infinity) - (b.event_origin ?? Infinity) || a.id.localeCompare(b.id));
  const members: Member[] = [];
  const memberIds = new Set<string>();
  const add = (nodeId: string, introducedBy: GraphDecision) => {
    const node = nodes.get(nodeId);
    if (!node || !RECORD_KINDS.has(node.kind) || memberIds.has(nodeId)) return;
    memberIds.add(nodeId);
    members.push({ node, raw: nodeRawId(nodeId), introducedBy, order: members.length + 1 });
  };
  for (const decision of own) {
    const decisionNode = `Decision:${decision.id}`;
    add(decisionNode, decision);
    const options = targets(decisionNode, "HAS_OPTION", "CHOSE");
    options.forEach(id => add(id, decision));
    targets(decisionNode, "BASED_ON", "PREMISED_ON_DIRECT", "ANSWERS").forEach(id => add(id, decision));
    options.forEach(id => targets(id, "PREMISED_ON").forEach(premise => add(premise, decision)));
  }
  // One hop from evidence and hypotheses already in: what supports or refutes them, and what they bear on.
  for (const member of [...members].filter(item => item.node.kind === "Evidence" || item.node.kind === "Hypothesis")) {
    for (const edge of [...(outgoing.get(member.node.id) ?? []), ...(incoming.get(member.node.id) ?? [])]) {
      if (edge.relation === "SUPPORTS" || edge.relation === "REFUTES") { add(edge.source, member.introducedBy); add(edge.target, member.introducedBy); }
    }
  }

  const contextId = `context:loomtracer:${projectRef}`;
  const id = bundleId(tenant, projectRef);
  const proposers = (decision: GraphDecision) => targets(`Decision:${decision.id}`, ...PROPOSER_EDGES).map(nodeRawId);
  const proposerOf = (decision: GraphDecision) => proposers(decision)[0] ?? decision.deciders[0]?.id ?? LEDGER_ACTOR_ID;
  const actors = new Map<string, Actor>([[LEDGER_ACTOR_ID, { id: LEDGER_ACTOR_ID, name: "Loomtracer ledger", kind: "system" }]]);
  const noteActor = (actorId: string) => { if (!actors.has(actorId)) actors.set(actorId, { id: actorId, name: actorId, kind: actorKind(actorId) }); return actorId; };

  const sources: SourceMessage[] = [];
  const entries: Entry[] = [];
  const spanOf = new Map<string, { messageId: string; start: number; end: number }>();
  const entryIdOf = new Map<string, string>();
  const chosen = new Set(edges.filter(edge => edge.relation === "CHOSE").map(edge => edge.target));
  for (const member of members) {
    const { node, raw, introducedBy, order } = member;
    const decision = node.kind === "Decision" ? decisions.get(raw) : undefined;
    const actorId = noteActor(proposerOf(introducedBy));
    const when = introducedBy.decided_at;
    const words = (decision ? decision.title : node.title ?? node.label ?? raw).trim() || raw;
    const stated = decision ? [words, decision.rationale?.trim()].filter(Boolean).join("\n\n") : words;
    const extra = decision ? [decision.quote && `Said: ${decision.quote.trim()}`, decision.question && `Answering: ${decision.question.trim()}`, decision.binds && `Binds: ${decision.binds.trim()}`, decision.undo_cost && `Undo cost: ${decision.undo_cost.trim()}`].filter((line): line is string => Boolean(line)) : [];
    const content = [stated, ...extra].join("\n\n");
    const sourceId = `source:${raw}`;
    const sourceRef = `loomtracer://${tenant}/${raw}${decision?.event_origin != null ? `#event-${decision.event_origin}` : ""}`;
    sources.push({ id: sourceId, originalId: raw, actorId, role: "record", content, occurredAt: when, loggedAt: when, order, sourceRef, attachmentIds: [], active: true, provenance: { kind: "authored", recordedBy: actorId, recordedAt: when ?? now } });
    const span = { messageId: sourceId, start: 0, end: stated.length };
    spanOf.set(node.id, span);
    entryIdOf.set(node.id, raw);
    const labels = [LABEL[node.kind]!];
    if (decision) labels.push(`status:${decision.status}`);
    if (node.kind === "Option") labels.push(chosen.has(node.id) ? "chosen" : "not chosen");
    entries.push({ id: raw, logicalId: raw, version: 1, contextId, labels, provenance: { expressedBy: actorId, recordedBy: actorId, occurredAt: when, recordedAt: when ?? now, basis: "direct", sourceSpans: [span] } });
    if (decision) for (const person of [...decision.deciders.map(item => item.id), ...targets(node.id, "REJECTED_BY", "ACCEPTED_BY", "PARTICIPATED_BY", "INITIATED_BY").map(nodeRawId)]) noteActor(person);
  }

  // Edges between two records are connections; an edge to another project's decision keeps a pointer.
  const external = new Map<string, ExternalEntryRef>();
  const externalFor = (nodeId: string): string | null => {
    const decision = nodeKind(nodeId) === "Decision" ? decisions.get(nodeRawId(nodeId)) : undefined;
    if (!decision || projectOf(decision) === projectRef) return null;
    const refId = `external:${decision.id}`;
    if (!external.has(refId)) external.set(refId, { id: refId, namespace: { datasetId: bundleId(tenant, projectOf(decision)), workspaceId: tenant, projectId: projectOf(decision) }, entryId: decision.id, originalId: decision.id });
    return refId;
  };
  const connections: Connection[] = [];
  const memberByNode = new Map(members.map(member => [member.node.id, member]));
  for (const edge of edges) {
    const sourceMember = memberByNode.get(edge.source), targetMember = memberByNode.get(edge.target);
    if (!sourceMember && !targetMember) continue;
    const sourceEntry = sourceMember ? entryIdOf.get(edge.source)! : externalFor(edge.source);
    const targetEntry = targetMember ? entryIdOf.get(edge.target)! : externalFor(edge.target);
    if (!sourceEntry || !targetEntry || sourceEntry === targetEntry) continue;
    const relation = edge.relation.toLowerCase();
    // The claim is the source's. When that is another project's decision, it is that decision's
    // proposer who made it, and the only text here to anchor it to is the local end's.
    const claimant = sourceMember ? sourceMember.introducedBy : decisions.get(nodeRawId(edge.source))!;
    const anchor = (sourceMember ?? targetMember)!;
    const actorId = noteActor(proposerOf(claimant));
    const connectionId = `connection:${relation}:${sourceEntry}:${targetEntry}`;
    connections.push({
      id: connectionId, logicalId: connectionId, version: 1, contextId, relation: `legacy:${relation}`,
      participants: [{ entryId: sourceEntry, role: "source" }, { entryId: targetEntry, role: "target" }],
      provenance: { expressedBy: actorId, recordedBy: actorId, occurredAt: claimant.decided_at, recordedAt: claimant.decided_at ?? now, basis: "direct", sourceSpans: [spanOf.get(anchor.node.id)!] },
    });
  }

  const head = String(Math.max(0, ...graph.decisions.map(decision => decision.event_origin ?? 0)));
  const runId = `run:${id}@${head}`;
  const bundle: ReasoningBundle = {
    schemaVersion: "0.3.0", id, title: project.name, namespace: { datasetId: id, workspaceId: tenant, projectId: projectRef },
    actors: [...actors.values()], contexts: [{ id: contextId, title: project.name, kind: "ledger_record" }], sources, attachments: [],
    manifest: {
      threadId: id, frozenAt: now, cutoffDescription: `Loomtracer ledger through event ${head}, read at ${now}.`,
      segments: sources.map(source => ({ ref: source.sourceRef, sha256: sha256(source.content), selectedMessageIds: [source.id] })), exclusions: [],
    },
    entries, connections,
    runs: sources.length ? [{
      id: runId, parentId: null, createdAt: now, recordedBy: LEDGER_ACTOR_ID, sourceCutoff: sources.length, sourceMessageIds: sources.map(source => source.id),
      addedEntryIds: entries.map(entry => entry.id), addedConnectionIds: connections.map(connection => connection.id), replacements: [],
      uncertainties: [
        "Drawn from the Loomtracer graph: decisions with their options, evidence, assumptions and questions. Requests, blockers, notifications, reaffirmations and project links are not drawn here.",
        ...(graph.truncated ? [`Only the newest decisions of this project are drawn in full; it holds ${graph.total_matches ?? "more"}. Older ones appear only where a shown decision links to them, and without what they weighed.`] : []),
      ],
      notes: `Rendered from GET /v1/graph?project=${projectRef} at ledger event ${head}. Each record is one Loomtracer node; each connection is one graph edge in its stored direction. Order is the order of the decision that introduced the record. Status and deciders are the graph's derived fields.`,
      reviewResolutions: [],
    }] : [],
    reviewNotes: [], authorityGrants: [], commitmentReceipts: [], externalEntryRefs: [...external.values()],
  };
  return { bundle, head };
}
