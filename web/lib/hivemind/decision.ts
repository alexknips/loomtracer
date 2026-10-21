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
  const unique = <T>(items: T[]) => [...new Set(items)];
  const uniqueRefs = (items: DecisionRef[]) => [...new Map(items.map(item => [item.id, item])).values()];
  const actors = (relation: string) => unique(claims(relation).filter(claim => claim.source === me).map(claim => nodeRawId(claim.target))).sort();
  const proposedBy = actors("PROPOSED_BY")[0];
  const acceptedBy = actors("ACCEPTED_BY"), rejectedBy = actors("REJECTED_BY");
  return {
    id: brief.id, title: brief.title, status: brief.status, rationale: brief.rationale,
    project: brief.project ? { ref: brief.project, label: brief.project_label ?? brief.project } : null,
    decidedAt: row?.decided_at ?? timeline.decided_at, proposedBy: proposedBy ?? null,
    deciders: row?.deciders ?? [],
    quote: row?.quote ?? null, question: row?.question ?? null, topics: brief.topic_keys,
    options: brief.option_ids.map(id => ({ id, label: label("Option", id), chosen: id === brief.chosen_option_id })),
    assumptions: brief.hypotheses.map(({ id, status, kind }) => ({ id, status, kind, statement: label("Hypothesis", id) })),
    evidence: brief.evidence_ids.map(id => ({ id, text: label("Evidence", id) })),
    premises: brief.premise_decision_ids.map(ref),
    binds: brief.binds ?? null, undoCost: brief.undo_cost ?? null,
    contest: acceptedBy.length && rejectedBy.length ? { acceptedBy, rejectedBy } : null,
    supersedes: uniqueRefs(claims("SUPERSEDES").filter(claim => claim.source === me).map(claim => ref(nodeRawId(claim.target)))),
    supersededBy: uniqueRefs(claims("SUPERSEDES").filter(claim => claim.target === me).map(claim => ref(nodeRawId(claim.source)))),
    timeline: timeline.entries,
  };
}
