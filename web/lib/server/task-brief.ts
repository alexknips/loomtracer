import { createHash } from "node:crypto";
import type { GraphEdgeRecord, GraphNodeRecord, ProjectRecord } from "@/types/graph";

import { ancestors, resolveProject, selectProjectNodes } from "../projects.ts";

export interface BriefRequest {
  project_ref: string;
  task: string;
  workflow_ref?: string;
  target_ref?: string;
  context_refs?: string[];
  previous_fingerprint?: string;
  delivery?: "full" | "deduplicated" | "compact";
  max_response_bytes?: number;
  max_linked_nodes?: number;
  /** Opt-in pre-expansion of task-relevant unlinked observations; default 0. */
  max_task_evidence_nodes?: number;
  baseline_token?: string;
  baseline_confirmed?: boolean;
  allow_delta?: boolean;
}

export interface BriefSnapshot {
  nodes: GraphNodeRecord[];
  edges: GraphEdgeRecord[];
  reference_edges?: BriefReferenceEdge[];
  include_reference_edges?: boolean;
  projects?: ProjectRecord[];
  complete: boolean;
  warnings: string[];
}

/** External references are discovery notices, never a carrier for foreign prose. */
export type BriefReferenceEdge = Pick<GraphEdgeRecord, "id" | "workspace_id" | "source_id" | "target_id" | "edge_type"> & { proposed: boolean };

function canonical(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonical);
  if (value && typeof value === "object") return Object.fromEntries(
    Object.entries(value).sort(([a], [b]) => a.localeCompare(b)).map(([k, v]) => [k, canonical(v)]),
  );
  return value;
}

export function recordHash(value: unknown): string {
  return createHash("sha256").update(JSON.stringify(canonical(value))).digest("hex");
}

export function scopedSnapshot(snapshot: BriefSnapshot, workspaceId: string, projectRef: string,
  includeReferenceEdges = snapshot.include_reference_edges === true): BriefSnapshot {
  const workspaceNodes = snapshot.nodes.filter(n => n.workspace_id === workspaceId);
  const workspaceProjects = snapshot.projects?.filter(p => p.workspace_id === workspaceId);
  const project = workspaceProjects ? resolveProject(workspaceProjects, projectRef) : undefined;
  if (workspaceProjects && !project) return { nodes: [], edges: [], projects: [], complete: false,
    warnings: [...snapshot.warnings, "Project reference not found. Select a project from list_projects before drafting."] };
  const nodes = workspaceProjects ? selectProjectNodes(workspaceNodes, workspaceProjects, projectRef, true)
    : workspaceNodes.filter(n => n.scope_refs.includes(projectRef));
  const projects = project && workspaceProjects ? [project, ...ancestors(workspaceProjects, project.id)] : undefined;
  const ids = new Set(nodes.map(n => n.id));
  const edges = snapshot.edges.filter(e => e.workspace_id === workspaceId && (includeReferenceEdges || e.edge_type === "supersedes")
    && ids.has(e.source_id) && ids.has(e.target_id));
  const referenceCandidates: BriefReferenceEdge[] = includeReferenceEdges ? [
    ...(snapshot.reference_edges ?? []),
    ...snapshot.edges.map(edge => ({ id: edge.id, workspace_id: edge.workspace_id, source_id: edge.source_id,
      target_id: edge.target_id, edge_type: edge.edge_type, proposed: edge.metadata.proposed === true })),
  ] : [];
  const reference_edges = [...new Map(referenceCandidates.filter(e => e.workspace_id === workspaceId
    && ids.has(e.source_id) !== ids.has(e.target_id)).map(edge => [edge.id, {
      id: edge.id, workspace_id: edge.workspace_id, source_id: edge.source_id,
      target_id: edge.target_id, edge_type: edge.edge_type, proposed: edge.proposed === true,
    }])).values()];
  const result: BriefSnapshot = { ...snapshot, nodes, edges, ...(projects ? { projects } : {}) };
  if (includeReferenceEdges) {
    result.reference_edges = reference_edges;
    result.include_reference_edges = true;
  } else {
    delete result.reference_edges;
    delete result.include_reference_edges;
  }
  return result;
}

export type ScopeQuestion = { node_id: string; dimension: string; options: string[] };

export function selectedBriefScope(request: Pick<BriefRequest, "workflow_ref" | "target_ref" | "context_refs">): string[] {
  return [...new Set([request.workflow_ref, request.target_ref, ...(request.context_refs ?? [])]
    .filter((s): s is string => Boolean(s)))].sort();
}

/** Shared applicability policy. Relevance ranking must never replace this check. */
export function nodeApplicability(node: Pick<GraphNodeRecord, "id" | "scope_refs">, request: Pick<BriefRequest, "workflow_ref" | "target_ref" | "context_refs">) {
  const selected = selectedBriefScope(request);
  const missing: ScopeQuestion[] = [];
  let mismatch = false;
  for (const dimension of ["workflow:", "target:", "trial:", "artifact:"]) {
    const requires = node.scope_refs.filter(ref => ref.startsWith(dimension));
    if (!requires.length) continue;
    const supplied = selected.filter(ref => ref.startsWith(dimension));
    if (supplied.length && !requires.some(ref => supplied.includes(ref))) mismatch = true;
    else if (!supplied.length) missing.push({ node_id: node.id, dimension, options: requires });
  }
  return { applicable: !mismatch && missing.length === 0, mismatch, missing: mismatch ? [] : missing };
}

export function snapshotFingerprint(snapshot: BriefSnapshot): string {
  return recordHash({
    ...(snapshot.projects ? { projects: [...snapshot.projects].sort((a, b) => a.id.localeCompare(b.id)) } : {}),
    nodes: snapshot.nodes.map(n => ({ ...n, scope_refs: [...n.scope_refs].sort() })).sort((a, b) => a.id.localeCompare(b.id)),
    edges: [...snapshot.edges].sort((a, b) => a.id.localeCompare(b.id)),
    ...(snapshot.include_reference_edges && snapshot.reference_edges?.length ? { reference_edges: [...snapshot.reference_edges].sort((a, b) => a.id.localeCompare(b.id)) } : {}),
  });
}

// Repeat the entire read, not just previously returned IDs: additions must be detected.
export async function readStableSnapshot(read: () => Promise<BriefSnapshot>): Promise<BriefSnapshot> {
  let previous = await read();
  for (let attempt = 0; attempt < 2; attempt++) {
    const current = await read();
    if (previous.complete && current.complete && snapshotFingerprint(previous) === snapshotFingerprint(current)) return current;
    previous = current;
  }
  return { ...previous, complete: false, warnings: [...previous.warnings, "Could not verify a stable, complete read; retry before drafting."] };
}

export function buildTaskBrief(snapshot: BriefSnapshot, workspaceId: string, request: BriefRequest, checkedAt: string) {
  const scoped = scopedSnapshot(snapshot, workspaceId, request.project_ref);
  const selected = selectedBriefScope(request);
  const groups: Record<"active_decisions" | "accepted_assumptions" | "unratified" | "observations" | "history", GraphNodeRecord[]> = {
    active_decisions: [], accepted_assumptions: [], unratified: [], observations: [], history: [],
  };
  const scopeQuestions: ScopeQuestion[] = [];
  let excluded = 0;
  const statusesInForce = new Set(["active", "accepted", "candidate", "proposed"]);
  for (const node of scoped.nodes) {
    const { mismatch, missing } = nodeApplicability(node, request);
    if (mismatch) { excluded++; continue; }
    if (missing.length) {
      if (statusesInForce.has(node.status)) scopeQuestions.push(...missing);
      excluded++;
      continue;
    }
    if (node.type === "Decision" && node.status === "active") groups.active_decisions.push(node);
    else if (node.type === "Assumption" && node.status === "accepted") groups.accepted_assumptions.push(node);
    else if (["candidate", "proposed"].includes(node.status)) groups.unratified.push(node);
    else if (node.status === "observation") groups.observations.push(node);
    else groups.history.push(node);
  }
  for (const records of Object.values(groups)) records.sort((a, b) => a.id.localeCompare(b.id));
  const fingerprint = snapshotFingerprint(scoped);
  const warnings = [...scoped.warnings];
  if (!scoped.nodes.length) warnings.push("No project records found in this workspace. Verify project and store; this is not clearance.");
  if (!groups.active_decisions.length) warnings.push("No applicable active Decisions. Unratified records are not approved guidance.");
  if (scopeQuestions.length) warnings.push("Some records need additional scope selection; do not apply them project-wide.");
  return {
    workspace_id: workspaceId, project_ref: request.project_ref, task: request.task,
    requested_scope: selected, checked_at: checkedAt, complete: scoped.complete,
    consistency: scoped.complete ? "repeat-read-verified; not a transaction or a guarantee against later changes" : "incomplete or unstable; not verified",
    fingerprint, changed: scoped.complete && request.previous_fingerprint ? fingerprint !== request.previous_fingerprint : null,
    coverage: { project_nodes: scoped.nodes.length, returned_nodes: Object.values(groups).reduce((sum, ns) => sum + ns.length, 0), excluded_by_scope: excluded },
    projects: scoped.projects,
    inherited_node_ids: scoped.nodes.filter(n => n.inherited_from_project_id).map(n => n.id),
    groups, scope_questions: scopeQuestions, warnings,
    record_versions: Object.fromEntries(scoped.nodes.map(n => [n.id, { updated_at: n.updated_at, hash: recordHash(n) }])),
    supersession_edges: scoped.edges.filter(edge => edge.edge_type === "supersedes"),
    relationships: scoped.edges,
    reference_edges: scoped.reference_edges ?? [],
    reference_notice: scoped.reference_edges?.length ? "These relationships reference records outside the selected project guidance. References do not enable inheritance or grant authority. Resolve the owning project and task scope before applying an external record." : null,
    advisory: true,
  };
}
