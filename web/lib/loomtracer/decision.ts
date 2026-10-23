import type { LoomtracerClient } from "./client";
import { nodeRawId, stored } from "./projects";
import type { DecisionBrief, DecisionStatus, GraphResponse, Timeline, TimelineEntry } from "./types";

type DecisionRef = { id: string; title: string; status: DecisionStatus };

/** Everything a person needs to read one decision: what it says, what it weighed, what it rests on, who stands where. */
export type DecisionDetail = {
  id: string; title: string; status: DecisionStatus; rationale: string | null;
  project: { ref: string; label: string } | null; decidedAt: string | null; proposedBy: string | null;
  deciders: { id: string; kind: string }[];
  quote: string | null; question: string | null; topics: string[];
  options: { id: string; label: string; chosen: boolean }[];
  /** The choice, when it is not one of `options`: a classifier draft can choose words it never weighed as an option. */
  chose: { id: string; label: string } | null;
  assumptions: { id: string; statement: string; status: string; kind: string }[];
  evidence: { id: string; text: string }[];
  premises: DecisionRef[];
  binds: { level: string; statement?: string } | null; undoCost: { level: string; statement?: string } | null;
  /** Both sides, whenever there are two. Shown whatever the status says: a disagreement is never folded away. */
  contest: { acceptedBy: string[]; rejectedBy: string[] } | null;
  supersedes: DecisionRef[]; supersededBy: DecisionRef[];
  timeline: TimelineEntry[];
};

/** `brief` is `GET /v1/decisions/{id}`: the decision asked for, even when something has replaced it. */
export function decisionDetail(brief: DecisionBrief, timeline: Timeline, graph: GraphResponse): DecisionDetail {
  const decisions = new Map(graph.decisions.map(decision => [decision.id, decision]));
  const nodes = new Map(graph.nodes.map(node => [node.id, node]));
  const ref = (id: string): DecisionRef => { const known = decisions.get(id); return { id, title: known?.title ?? id, status: known?.status ?? "proposed" }; };
  const row = decisions.get(brief.id);
  const me = `Decision:${brief.id}`;
  const claims = (relation: string) => graph.edges.filter(edge => edge.relation === relation && !edge.derived).map(stored);
  const label = (kind: string, id: string) => nodes.get(`${kind}:${id}`)?.title ?? nodes.get(`${kind}:${id}`)?.label ?? id;
  // The server returns each (relation, source, target) once, so nothing here collapses repeats.
  const actors = (relation: string) => claims(relation).filter(claim => claim.source === me).map(claim => nodeRawId(claim.target)).sort();
  const proposedBy = actors("PROPOSED_BY")[0];
  const acceptedBy = actors("ACCEPTED_BY"), rejectedBy = actors("REJECTED_BY");
  return {
    id: brief.id, title: brief.title, status: brief.status, rationale: brief.rationale,
    project: brief.project ? { ref: brief.project, label: brief.project_label ?? brief.project } : null,
    decidedAt: row?.decided_at ?? timeline.decided_at, proposedBy: proposedBy ?? null,
    deciders: row?.deciders ?? [],
    quote: row?.quote ?? null, question: row?.question ?? null, topics: brief.topic_keys,
    options: brief.option_ids.map(id => ({ id, label: label("Option", id), chosen: id === brief.chosen_option_id })),
    chose: brief.chosen_option_id && !brief.option_ids.includes(brief.chosen_option_id) ? { id: brief.chosen_option_id, label: label("Option", brief.chosen_option_id) } : null,
    assumptions: brief.hypotheses.map(({ id, status, kind }) => ({ id, status, kind, statement: label("Hypothesis", id) })),
    evidence: brief.evidence_ids.map(id => ({ id, text: label("Evidence", id) })),
    premises: brief.premise_decision_ids.map(ref),
    binds: brief.binds ?? null, undoCost: brief.undo_cost ?? null,
    contest: acceptedBy.length && rejectedBy.length ? { acceptedBy, rejectedBy } : null,
    supersedes: claims("SUPERSEDES").filter(claim => claim.source === me).map(claim => ref(nodeRawId(claim.target))),
    supersededBy: claims("SUPERSEDES").filter(claim => claim.target === me).map(claim => ref(nodeRawId(claim.source))),
    timeline: timeline.entries,
  };
}

/**
 * The decision id a `/decisions/[id]` path segment names. Next hands the segment over still percent-encoded, as the links
 * wrote it (`capture%3A105186%3A3` for `capture:105186:3`), and the client escapes an id itself, so the segment is decoded
 * once here; left as it is, the server is asked for `capture%253A105186%253A3`. A segment that does not decode names no
 * decision, so it is `null` and the page answers not found.
 */
export function decisionIdFromSegment(segment: string): string | null {
  try { return decodeURIComponent(segment); } catch { return null; }
}

/** What a decision page reads: the decision, its dated story and the graph around it, never the whole tenant graph. */
export async function readDecision(client: Pick<LoomtracerClient, "decision" | "timeline" | "decisionGraph">, id: string): Promise<DecisionDetail> {
  return decisionDetail(...await Promise.all([client.decision(id), client.timeline(id), client.decisionGraph(id)]));
}
