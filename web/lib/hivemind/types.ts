/** The slice of the HiveMind HTTP API this UI reads. Shapes follow docs/GRAPH_CONTRACT.md and the REST handlers. */
export type DecisionStatus = "proposed" | "accepted" | "rejected" | "contested" | "superseded";
export type ActorKind = "human" | "agent" | "unknown";

export type GraphDecision = {
  id: string; title: string; rationale: string | null; status: DecisionStatus;
  project: string | null; topic_keys: string[]; decided_at: string | null; event_origin: number | null;
  deciders: { id: string; kind: ActorKind }[]; slug: string | null;
  quote?: string | null; question?: string | null; source?: string | null; source_ref?: string | null;
  binds?: string | null; binds_level?: string | null; undo_cost?: string | null; undo_cost_level?: string | null;
};
/** `id` is `<Kind>:<node id>`; `label` is the words the node reads as (an option also has `title`). */
export type GraphNode = { id: string; kind: string; label?: string | null; title?: string | null; recorded_as?: string | null; decided_at?: string | null; slug?: string | null };
/** An arrow from the newer node to the older one. `reversed` says it runs against the stored direction. */
export type GraphEdge = { from: string; to: string; relation: string; label: string; reversed: boolean; derived?: boolean };
/**
 * `GET /v1/graph`. With `?project=` it is that project's page: its newest decisions and what hangs off them
 * (`limit` of them; `truncated` and `next_cursor` continue to older ones), plus the far end of every edge,
 * which may be a decision of another project. Without `project` the paging fields are absent.
 */
export type GraphResponse = {
  decisions: GraphDecision[]; nodes: GraphNode[]; edges: GraphEdge[];
  limit?: number; total_matches?: number; truncated?: boolean; next_cursor?: string | null;
};

/** `data` of the query-envelope endpoints (`{ result_count, truncated, latency_ms, data }`). */
export type Envelope<T> = { result_count: number; truncated: boolean; latency_ms: number; data: T };

export type DecisionBrief = {
  id: string; title: string; rationale: string | null; topic_keys: string[]; status: DecisionStatus;
  project: string | null; project_label: string | null; chosen_option_id: string | null; option_ids: string[]; evidence_ids: string[];
  hypotheses: { id: string; status: string; kind: string }[]; premise_decision_ids: string[];
  binds?: { level: string; statement?: string } | null; undo_cost?: { level: string; statement?: string } | null; floor?: string | null;
};
/** With a `project`, `scope` says how a decision reached the answer: its own project, or one it is part of or depends on. */
export type SearchItem = { decision: DecisionBrief; rank: number; matched_fields: string[]; scope?: { relation: "own" | "parent" | "dependency"; label: string } };
export type SearchPage = { query: string | null; total_matches: number; next_cursor: string | null; limit: number; items: SearchItem[]; scope?: { project: string; project_label: string; note: string } };

export type TimelineEntry = { event_origin: number; citation_id: string; ts: string; actor_id: string; kind: string; reason?: string };
export type Timeline = { decision_id: string; decided_at: string | null; entries: TimelineEntry[] };
export type SupersessionChain = { decision_ids: string[] };

export type AttentionItem = {
  decision_id: string; title: string; slug: string | null; project: string | null; project_label: string | null;
  status: DecisionStatus; decided_at?: string | null; captured_at?: string | null;
  contest?: { kind: string; accepted_by?: string[]; rejected_by?: string[] };
  rests_on?: { kind: string; id: string; label: string }[];
  binds?: { level: string; statement?: string } | null; undo_cost?: { level: string; statement?: string } | null; floor?: string | null;
};
export type AttentionPage = { total_matches: number; next_cursor: string | null; limit: number; items: AttentionItem[] };
/** A question someone asked that no decision answers yet. */
export type WaitingItem = { request_id: string; question_id: string; text: string; asked_at: string | null; requested_by: string };
export type WaitingPage = { total_matches: number; next_cursor: string | null; limit: number; items: WaitingItem[] };

export type Whoami = { actor_id: string; tenant_id: string };

export type DecisionCounts = Record<DecisionStatus, number> & { total: number };
/** One row of `GET /v1/projects`: a registered project, or a personal one that holds a decision. */
export type ProjectEntry = { handle: string; name: string; personal: boolean; parent: string | null; decisions: DecisionCounts };
export type ProjectsPage = { limit: number; cursor: string | null; next_cursor: string | null; total_matches: number; unfiled_decisions: number; items: ProjectEntry[] };

/** A project with its place in the part_of hierarchy. */
export type ProjectSummary = {
  ref: string; name: string; parent: string | null; children: string[];
  counts: DecisionCounts;
};
