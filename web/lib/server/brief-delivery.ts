import type { GraphEdgeRecord, GraphNodeRecord, ProjectRecord } from "@/types/graph";
import { recordHash, type BriefRequest, type BriefReferenceEdge, type ScopeQuestion } from "./task-brief";
import { briefScopeKey, diffBriefManifests, makeBriefManifest, usableBriefBaseline, type BriefBaselineStore } from "./brief-baseline";

export type BriefDeliveryNode = Omit<GraphNodeRecord, "workspace_id" | "created_by_actor_id" | "ratified_by_actor_id" | "ratified_by_user_id" | "actor"> & {
  source_project?: ProjectRecord | null;
  source_project_id?: string | null;
  created_by?: string | null;
  ratified_by?: string | null;
  inherited?: boolean;
};
export type BriefDeliveryEdge = Omit<GraphEdgeRecord, "workspace_id">;
export interface DeliverableBrief {
  fingerprint: string;
  project_ref: string;
  complete: boolean;
  groups: Record<string, BriefDeliveryNode[]>;
  scope_questions: ScopeQuestion[];
  warnings: string[];
  projects?: ProjectRecord[];
  supersession_edges: BriefDeliveryEdge[];
  relationships?: BriefDeliveryEdge[];
  reference_edges?: Array<Omit<BriefReferenceEdge, "workspace_id">>;
  record_versions?: unknown;
  [key: string]: unknown;
}

const REQUIRED_GROUPS = new Set(["active_decisions", "accepted_assumptions"]);
// Preserve material counter-evidence and premises, including unratified records.
// Inclusion does not promote a relationship or its endpoint to approved guidance.
const PROTECTED_RELATIONS = new Set(["supersedes", "contradicts", "invalidates", "depends_on", "based_on_assumption"]);
const MAX_BYTES = 100000;
export function serializedBriefBytes(value: unknown) { return Buffer.byteLength(JSON.stringify(value), "utf8"); }

function deduplicateNode(node: BriefDeliveryNode, projects: ProjectRecord[] = []): BriefDeliveryNode {
  const { source_project, ...rest } = node;
  // Preserve nulls and standalone metadata when there is no identical lookup row.
  const lookup = source_project && projects.find(project => project.id === source_project.id);
  return source_project && lookup && recordHash(lookup) === recordHash(source_project)
    ? { ...rest, source_project_id: source_project.id } : node;
}

function protectedNeighbors(relationships: BriefDeliveryEdge[]) {
  const neighbors = new Map<string, string[]>();
  for (const edge of relationships) {
    if (!(PROTECTED_RELATIONS.has(edge.edge_type) || edge.metadata.material === true)) continue;
    neighbors.set(edge.source_id, [...(neighbors.get(edge.source_id) ?? []), edge.target_id]);
    neighbors.set(edge.target_id, [...(neighbors.get(edge.target_id) ?? []), edge.source_id]);
  }
  return neighbors;
}

function protectedClosure(seed: Iterable<string>, neighbors: Map<string, string[]>) {
  const required = new Set(seed);
  const queue = [...required];
  for (let i = 0; i < queue.length; i++) {
    for (const id of neighbors.get(queue[i]) ?? []) if (!required.has(id)) { required.add(id); queue.push(id); }
  }
  return required;
}

function relationPlan(brief: DeliverableBrief) {
  const records = Object.values(brief.groups).flat();
  const byId = new Map(records.map(node => [node.id, node]));
  const relationships = brief.relationships ?? brief.supersession_edges;
  const neighbors = protectedNeighbors(relationships);
  const governing = new Set(Object.entries(brief.groups).filter(([group]) => REQUIRED_GROUPS.has(group)).flatMap(([, nodes]) => nodes.map(node => node.id)));
  const supporting = relationships.filter(edge => edge.edge_type === "supports" && (governing.has(edge.source_id) || governing.has(edge.target_id)))
    .flatMap(edge => [edge.source_id, edge.target_id]);
  // A supporting record can contain an exception absent from its summary. Without
  // an authored materiality declaration, preserve its full body conservatively.
  const required = protectedClosure([...governing, ...supporting], neighbors);
  const missing = [...required].filter(id => !byId.has(id));
  const optional = [...new Set(relationships.flatMap(edge => {
    if (required.has(edge.source_id) && byId.has(edge.target_id) && !required.has(edge.target_id)) return [edge.target_id];
    if (required.has(edge.target_id) && byId.has(edge.source_id) && !required.has(edge.source_id)) return [edge.source_id];
    return [];
  }))].sort();
  return { required, missing, optional, relationships, records, neighbors };
}

function indexEntry(node: BriefDeliveryNode) {
  const source = typeof node.metadata.source_ref === "string" ? node.metadata.source_ref : null;
  return {
    id: node.id, title: node.title.slice(0, 180), type: node.type, status: node.status,
    summary_preview: node.summary.slice(0, 240), scope_type: node.scope_type,
    scope_refs: node.scope_refs.slice(0, 12).map(ref => ref.slice(0, 160)),
    source_ref: source?.slice(0, 240) ?? null, updated_at: node.updated_at,
    version: recordHash(node), partial: true,
    preview_truncated: node.title.length > 180 || node.summary.length > 240 || node.scope_refs.length > 12
      || node.scope_refs.some(ref => ref.length > 160) || (source?.length ?? 0) > 240,
  };
}

function indexPriority(node: BriefDeliveryNode, task: string) {
  const terms = [...new Set(task.toLowerCase().match(/[a-z0-9_-]{3,}/g) ?? [])];
  const text = `${node.title} ${node.summary} ${node.body}`.toLowerCase();
  return terms.reduce((score, term) => score + Number(text.includes(term)), 0);
}

// Discovery words alone are weak signals: "current investigation evidence" must
// not cause arbitrary observations to be sent in full. This is a conservative
// lexical prefetch, not a semantic applicability or authority decision.
const GENERIC_TASK_TERMS = new Set(("about after again against also another any are because been before being between both can could "
  + "does each for from had has have how into its may more most must not now only other our out same should some such than that the "
  + "their them then there these they this those through under until use used using very was were what when where which while who will with would your "
  + "assess approved architecture body brief briefing check complete constraint constraints context current currently decision decisions "
  + "evidence exact explain finding findings identify inspect investigation investigations maintain maintenance need needed plan planning "
  + "prepare prior project projects provide record records report reports result results return review scope source sources state status "
  + "summary task tasks test testing tests").split(/\s+/));

function taskEvidencePriority(node: BriefDeliveryNode, task: string) {
  const terms = (text: string) => new Set((text.toLowerCase().match(/[a-z0-9]+(?:[-_][a-z0-9]+)*/g) ?? [])
    .filter(term => term.length >= 3 && /[a-z]/.test(term) && !GENERIC_TASK_TERMS.has(term)));
  const title = terms(node.title), summary = terms(node.summary);
  return [...terms(task)].reduce((score, term) => score + (title.has(term) ? 2 : summary.has(term) ? 1 : 0), 0);
}

function boundedIndex(brief: DeliverableBrief, required: Set<string>, missing: string[], budget: number, reason: string) {
  const ids = [...required].sort();
  const result = {
    project_ref: brief.project_ref, fingerprint: brief.fingerprint, delivery: "index", complete: false,
    source_complete: brief.complete, guidance_complete: false, archive_complete: false,
    reason, max_response_bytes: budget,
    coverage: { required_nodes: ids.length, delivered_nodes: 0, missing_linked_nodes: missing.length },
    expansion_required: ids.slice(0, 50), expansion_list_truncated: ids.length > 50,
    instructions: "This index is not guidance. Expand the required records and their material linked context, or request a larger supported byte budget. Resolve scope questions before applying guidance. No reusable baseline was issued.",
  };
  while (serializedBriefBytes(result) > budget && result.expansion_required.length) {
    result.expansion_required.pop(); result.expansion_list_truncated = true;
  }
  if (serializedBriefBytes(result) <= budget) return result;
  return { delivery: "index", complete: false, guidance_complete: false, reason: "Required guidance exceeds the response byte budget. This is not clearance; expand the project records. No reusable baseline was issued." };
}

/** Prepare a bounded logical payload; MCP transport and client token accounting are separate. */
export async function deliverTaskBrief(brief: DeliverableBrief, request: BriefRequest, options: {
  workspaceId: string;
  /** Bind to the authenticated connection AND actor, not a caller-supplied label. */
  principalId: string;
  baselineStore?: BriefBaselineStore;
  now?: number;
}): Promise<Record<string, unknown>> {
  const budget = Math.min(MAX_BYTES, Math.max(1024, request.max_response_bytes ?? MAX_BYTES));
  const mode = request.delivery ?? "full";
  const plan = relationPlan(brief);
  const selected = mode !== "compact" ? new Set(plan.records.map(node => node.id)) : new Set([...plan.required].filter(id => !plan.missing.includes(id)));
  const originalCoverage = typeof brief.coverage === "object" && brief.coverage ? brief.coverage : {};
  const warnings = [...brief.warnings];
  if (plan.missing.length) warnings.push("Material linked guidance is outside the resolved scope or unavailable. Expand the referenced records and resolve their scope before applying guidance.");
  const scopeResolved = brief.scope_questions.length === 0 && plan.missing.length === 0;
  const makePayload = () => {
    const groups = Object.fromEntries(Object.entries(brief.groups).map(([group, nodes]) => [group, nodes.filter(node => selected.has(node.id)).map(node => mode === "full" ? node : deduplicateNode(node, brief.projects))]));
    const relationships = plan.relationships.filter(edge => selected.has(edge.source_id) && selected.has(edge.target_id));
    const delivered = Object.values(groups).reduce((sum, nodes) => sum + nodes.length, 0);
    const result = {
      ...brief, groups, relationships, supersession_edges: relationships.filter(edge => edge.edge_type === "supersedes"),
      warnings, complete: brief.complete && scopeResolved, source_complete: brief.complete,
      guidance_complete: brief.complete && scopeResolved, archive_complete: brief.complete && delivered === plan.records.length,
      delivery: mode, max_response_bytes: budget, expansion_required: plan.missing,
      coverage: { ...originalCoverage, available_nodes: plan.records.length, returned_nodes: delivered,
        required_nodes: plan.required.size, optional_nodes_omitted: plan.records.filter(node => !selected.has(node.id) && !plan.required.has(node.id)).length,
        available_relationships: plan.relationships.length, returned_relationships: relationships.length },
      record_index: [] as ReturnType<typeof indexEntry>[], index_complete: mode !== "compact", index_omitted_count: 0,
      index_instructions: mode === "compact" ? "Index entries are partial discovery previews, not complete guidance. Expand a relevant ID before relying on its evidence or scope. Use project-scoped get_relevant_context or search when the index is truncated; unlinked findings remain searchable." : undefined,
    };
    // Record hashes are retained server-side; repeat copies defeat compact delivery.
    if (mode === "compact") delete result.record_versions;
    return result;
  };
  // Leave space for the opaque token, expiry and continuation warning.
  const reserve = 768;
  let payload = makePayload();
  if (serializedBriefBytes(payload) + reserve > budget) return boundedIndex(brief, plan.required, plan.missing, budget, "Required indivisible records or full delivery exceed the byte budget.");
  if (mode === "compact") {
    const knownIds = new Set(plan.records.map(node => node.id));
    const linkedLimit = Math.min(50, Math.max(0, request.max_linked_nodes ?? 8));
    for (const id of plan.optional) {
      if ([...selected].filter(nodeId => !plan.required.has(nodeId)).length >= linkedLimit) break;
      const closure = protectedClosure([id], plan.neighbors);
      if ([...closure].some(nodeId => !knownIds.has(nodeId))) continue;
      const added = [...closure].filter(nodeId => !selected.has(nodeId));
      if ([...selected, ...added].filter(nodeId => !plan.required.has(nodeId)).length > linkedLimit) continue;
      for (const nodeId of added) selected.add(nodeId);
      const candidate = makePayload();
      if (serializedBriefBytes(candidate) + reserve > budget) for (const nodeId of added) selected.delete(nodeId);
      else payload = candidate;
    }
    const taskEvidenceLimit = Math.min(8, Math.max(0, Math.floor(request.max_task_evidence_nodes ?? 0)));
    if (taskEvidenceLimit) {
      const linkedIds = new Set([...plan.relationships, ...(brief.reference_edges ?? [])]
        .flatMap(edge => [edge.source_id, edge.target_id]));
      const candidates = (brief.groups.observations ?? [])
        .filter(node => node.type === "Attempt" && node.status === "observation" && !selected.has(node.id) && !linkedIds.has(node.id))
        .map(node => ({ node, priority: taskEvidencePriority(node, request.task) }))
        .filter(candidate => candidate.priority > 0)
        .sort((a, b) => b.priority - a.priority || a.node.id.localeCompare(b.node.id));
      let addedCount = 0;
      for (const { node } of candidates) {
        const closure = protectedClosure([node.id], plan.neighbors);
        if ([...closure].some(id => !knownIds.has(id))) continue;
        const added = [...closure].filter(id => !selected.has(id));
        if (!added.length || addedCount + added.length > taskEvidenceLimit) continue;
        for (const id of added) selected.add(id);
        const candidate = makePayload();
        if (serializedBriefBytes(candidate) + reserve > budget) for (const id of added) selected.delete(id);
        else { payload = candidate; addedCount += added.length; }
        if (addedCount >= taskEvidenceLimit) break;
      }
    }
    const omitted = plan.records.filter(node => !selected.has(node.id)).map(node => ({ node, priority: indexPriority(node, request.task) }))
      .sort((a, b) => b.priority - a.priority || a.node.id.localeCompare(b.node.id));
    for (const { node } of omitted) {
      const entry = indexEntry(node);
      payload.record_index.push(entry);
      if (serializedBriefBytes(payload) + reserve > budget) payload.record_index.pop();
    }
    payload.index_complete = payload.record_index.length === omitted.length;
    payload.index_omitted_count = omitted.length - payload.record_index.length;
  }
  const scopeKey = briefScopeKey(options.workspaceId, options.principalId, request);
  const manifest = makeBriefManifest(payload, scopeKey);
  let baseline = null;
  const baselineWarnings: string[] = [];
  if (request.baseline_token && request.baseline_confirmed === true && options.baselineStore && payload.complete) {
    try {
      const candidate = await options.baselineStore.get(request.baseline_token, request.project_ref, scopeKey);
      if (usableBriefBaseline(candidate, request.baseline_token, scopeKey, options.now ?? Date.now())) baseline = candidate;
      else baselineWarnings.push("The retained baseline is unavailable, expired, or belongs to a different scope. A complete payload is returned.");
    } catch { baselineWarnings.push("Baseline storage is unavailable. A complete payload is returned."); }
  }
  let response: Record<string, unknown> = payload;
  if (baseline) {
    const diff = diffBriefManifests(baseline.manifest, manifest);
    if (baseline.manifest.fingerprint === manifest.fingerprint && !diff.metadata_changed
      && !diff.changed_node_ids.length && !diff.removed_node_ids.length && !diff.changed_relationship_ids.length && !diff.removed_relationship_ids.length) {
      response = {
        project_ref: brief.project_ref, fingerprint: brief.fingerprint, checked_at: brief.checked_at,
        delivery: "unchanged", complete: true, guidance_complete: true, changed: false,
        baseline: { token: baseline.token, expires_at: baseline.expires_at },
        instructions: "Reuse only the complete baseline you explicitly confirmed retaining for this exact task scope.",
      };
      return response;
    }
    if (request.allow_delta === true) {
      const changedNodes = new Set(diff.changed_node_ids);
      const changedRelationships = new Set(diff.changed_relationship_ids);
      response = {
        ...payload, delivery: "delta", changed: true, base_token: baseline.token,
        groups: Object.fromEntries(Object.entries(payload.groups).map(([group, nodes]) => [group, nodes.filter(node => changedNodes.has(node.id))])),
        relationships: payload.relationships.filter(edge => changedRelationships.has(edge.id)),
        supersession_edges: payload.supersession_edges.filter(edge => changedRelationships.has(edge.id)),
        removed_node_ids: diff.removed_node_ids, removed_relationship_ids: diff.removed_relationship_ids,
        metadata_changed: diff.metadata_changed,
        instructions: "Apply removals first. Replace each changed node by ID across all groups, then replace relationships by ID. Replace envelope metadata. Reuse unchanged records only from the confirmed base_token. The merged result, not this delta alone, is the complete briefing.",
      };
      if (serializedBriefBytes(response) + reserve > budget) response = payload;
    }
  }
  if (payload.complete && options.baselineStore) {
    try { response = { ...response, baseline: await options.baselineStore.save(request.project_ref, scopeKey, manifest) }; }
    catch { baselineWarnings.push("Baseline storage is unavailable; retain the returned briefing or apply this delta to its confirmed baseline. No continuation token was issued."); }
  }
  if (baselineWarnings.length) response = { ...response, baseline_warnings: baselineWarnings };
  if (serializedBriefBytes(response) > budget) return boundedIndex(brief, plan.required, plan.missing, budget, "Delivery metadata exceeds the byte budget.");
  return response;
}
