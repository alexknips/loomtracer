import { randomUUID } from "node:crypto";
import { agentClient, type AgentContext } from "@/lib/server/agent-auth";
import { demoGraph } from "@/lib/demo";
import { isDemoMode } from "@/lib/env";
import type { GraphEdgeRecord, GraphNodeRecord, ProjectRecord } from "@/types/graph";
import { buildTaskBrief, readStableSnapshot, scopedSnapshot, type BriefRequest, type BriefSnapshot } from "@/lib/server/task-brief";

import { selectProjectNodes, resolveProject, PROJECT_REF, ancestors } from "@/lib/projects";
import { readWorkspaceRows } from "@/lib/server/workspace";
import { pilotFeatureEnabled, requirePilot } from "./pilot-features";
import { captureRelationship, operationId, relationshipProposal, type CaptureRelationship, type RelationshipProposal } from "./relationship-input";
import { deliverTaskBrief } from "./brief-delivery";
import { durableBriefBaselineStore } from "./brief-baseline-store";
import { boundedBriefsEnabled, withBriefDefaults } from "./memory-defaults";

type ToolNode = Omit<GraphNodeRecord, "workspace_id" | "created_by_actor_id" | "ratified_by_actor_id" | "ratified_by_user_id" | "actor"> & {
  created_by: string | null;
  ratified_by: string | null;
};

export interface CaptureInput {
  project_ref?: string;
  title: string;
  summary?: string;
  body?: string;
  scope_type?: string | null;
  scope_refs?: string[];
  confidence?: string | null;
  reversibility?: string | null;
  consequence?: string | null;
  reconsider_conditions?: string[];
  chosen_option?: string | null;
  rejected_options?: string[];
  evidence_summary?: string;
  source_ref?: string;
  outcome?: string;
  operation_id?: string;
  relationships?: CaptureRelationship[];
}

const STATUS_WEIGHTS: Record<string, number> = { active: 8, accepted: 8, candidate: 5, observation: 4, proposed: 4, superseded: 2, rejected: 2, invalidated: 1, archived: 1 };
const TYPE_WEIGHTS: Record<string, number> = { Decision: 4, Attempt: 3, Assumption: 3, Evidence: 2, Option: 1, Artifact: 1, Scope: 1 };
const STOPWORDS = new Set(["a", "an", "and", "are", "as", "be", "by", "for", "from", "in", "into", "is", "it", "of", "on", "or", "that", "the", "this", "to", "use", "with"]);
const PREFIXES: Record<string, string> = { Decision: "D", Option: "O", Evidence: "E", Assumption: "A", Attempt: "T", Artifact: "ART", Scope: "S" };

function tokenize(text: string): string[] {
  return (text.toLowerCase().match(/[a-z0-9_][a-z0-9_.-]*/g) ?? []).filter((token) => token.length >= 3 && !STOPWORDS.has(token));
}

function corpus(node: GraphNodeRecord, actorLabels: Map<string, string>): string {
  const creator = node.created_by_actor_id ? actorLabels.get(node.created_by_actor_id) ?? "" : "";
  const ratifier = node.ratified_by_actor_id ? actorLabels.get(node.ratified_by_actor_id) ?? "" : "";
  return [node.id, node.type, node.status, node.title, node.summary, node.body, node.scope_type ?? "", node.scope_refs.join(" "), node.confidence ?? "", node.reversibility ?? "", node.consequence ?? "", node.reconsider_conditions.join(" "), node.actor?.display_name ?? "", creator, ratifier, JSON.stringify(node.metadata)].join("\n").toLowerCase();
}

export function rankHostedNodes(nodes: GraphNodeRecord[], query: string, limit = 10, actorLabels = new Map<string, string>()): Array<{ node: GraphNodeRecord; score: number }> {
  const tokens = tokenize(query);
  if (!tokens.length) return [];
  return nodes.map((node) => {
    const text = corpus(node, actorLabels);
    let score = tokens.reduce((total, token) => total + (text.includes(token) ? 3 : 0), 0);
    if (!score) return null;
    score += STATUS_WEIGHTS[node.status] ?? 0;
    score += TYPE_WEIGHTS[node.type] ?? 0;
    if (node.scope_refs.length && tokens.some((token) => node.scope_refs.join(" ").toLowerCase().includes(token))) score += 3;
    return { node, score };
  }).filter((item): item is { node: GraphNodeRecord; score: number } => item !== null)
    .sort((a, b) => b.score - a.score || b.node.updated_at.localeCompare(a.node.updated_at)).slice(0, limit);
}

function toolNode(node: GraphNodeRecord, actorLabel: string | null = null, ratifierLabel: string | null = null): ToolNode {
  // Database rows can include generated storage fields such as search_vector.
  // Project the tool contract explicitly while retaining all authored metadata.
  return {
    id: node.id, type: node.type, status: node.status, title: node.title,
    summary: node.summary, body: node.body,
    scope_type: node.scope_type, scope_refs: node.scope_refs,
    confidence: node.confidence, reversibility: node.reversibility, consequence: node.consequence,
    reconsider_conditions: node.reconsider_conditions, metadata: node.metadata,
    created_at: node.created_at, updated_at: node.updated_at, ratified_at: node.ratified_at,
    project_id: node.project_id, inheritance_mode: node.inheritance_mode,
    inherited_from_project_id: node.inherited_from_project_id,
    created_by: actorLabel, ratified_by: ratifierLabel ?? node.ratified_by_user_id,
  };
}

function toolEdge(edge: GraphEdgeRecord) {
  return { id: edge.id, source_id: edge.source_id, target_id: edge.target_id,
    edge_type: edge.edge_type, summary: edge.summary, metadata: edge.metadata, created_at: edge.created_at };
}

function compact(node: GraphNodeRecord, score?: number): string {
  const status = ["observation", "candidate", "proposed"].includes(node.status) ? `${node.status} (unratified)` : node.status;
  const lines = [`- ${node.id} [${node.type}; ${status}] ${node.title}`];
  if (node.summary) lines.push(`  Summary: ${node.summary}`);
  if (node.scope_type || node.scope_refs.length) lines.push(`  Scope: ${node.scope_type ?? "unspecified"}: ${node.scope_refs.join(", ") || "unspecified"}`);
  if (node.confidence || node.reversibility || node.consequence) lines.push(`  Decision shape: confidence=${node.confidence ?? "unknown"}, reversibility=${node.reversibility ?? "unknown"}, consequence=${node.consequence ?? "unknown"}`);
  const rejected = Array.isArray(node.metadata.rejected_options) ? node.metadata.rejected_options.map(String) : [];
  if (rejected.length) lines.push(`  Rejected options: ${rejected.join(", ")}`);
  if (node.reconsider_conditions.length) lines.push(`  Reconsider if: ${node.reconsider_conditions.join("; ")}`);
  if (score !== undefined) lines.push(`  Relevance score: ${score}`);
  return lines.join("\n");
}

export class HostedGraph {
  constructor(private readonly agent: AgentContext) {}

  private get database() { return agentClient(this.agent.accessToken); }

  async listProjects(): Promise<ProjectRecord[]> {
    return isDemoMode() ? demoGraph.projects ?? [] : readWorkspaceRows<ProjectRecord>(this.database, "projects", this.agent.workspaceId);
  }

  async allNodes(projectRef?: string): Promise<GraphNodeRecord[]> {
    const nodes = isDemoMode() ? demoGraph.nodes : await readWorkspaceRows<GraphNodeRecord>(this.database, "nodes", this.agent.workspaceId);
    if (!projectRef) return nodes;
    return selectProjectNodes(nodes, await this.listProjects(), projectRef, true);
  }

  async actorLabels(ids: Array<string | null>): Promise<Map<string, string>> {
    const actorIds = [...new Set(ids.filter((value): value is string => Boolean(value)))];
    if (!actorIds.length) return new Map();
    if (isDemoMode()) return new Map(demoGraph.nodes.filter((node) => node.actor).map((node) => [node.actor!.id, node.actor!.display_name]));
    const { data, error } = await this.database.from("actors").select("id,display_name").eq("workspace_id", this.agent.workspaceId).in("id", actorIds);
    if (error) throw error;
    return new Map((data ?? []).map((actor) => [actor.id, actor.display_name]));
  }

  async search(query: string, limit = 10, projectRef?: string) {
    const nodes = await this.allNodes(projectRef);
    const labels = await this.actorLabels(nodes.flatMap((node) => [node.created_by_actor_id, node.ratified_by_actor_id]));
    const ranked = rankHostedNodes(nodes, query, limit, labels);
    return ranked.map(({ node, score }) => ({ node, score, toolNode: toolNode(
      node,
      node.created_by_actor_id ? labels.get(node.created_by_actor_id) ?? null : null,
      node.ratified_by_actor_id ? labels.get(node.ratified_by_actor_id) ?? null : null,
    ) }));
  }

  async relevantContext(task: string, files: string[], technologies: string[], proposedPlan: string, tokenBudget: number, projectRef?: string) {
    const matches = await this.search([task, proposedPlan, files.join(" "), technologies.join(" ")].join(" "), 12, projectRef);
    if (!matches.length) return { text: "No relevant Neurofilament decisions found.", matches: [] };
    const blocks = ["Relevant Neurofilament context:"];
    let words = blocks[0].split(/\s+/).length;
    for (const match of matches) {
      const block = compact(match.node, match.score);
      const blockWords = block.split(/\s+/).length;
      if (words + blockWords > tokenBudget && blocks.length > 1) break;
      blocks.push(block);
      words += blockWords;
    }
    return { text: blocks.join("\n\n"), matches };
  }

  private async briefSnapshot(projectRef: string): Promise<BriefSnapshot> {
    const includeRelationships = boundedBriefsEnabled() || pilotFeatureEnabled("briefs");
    if (isDemoMode()) return scopedSnapshot({ nodes: demoGraph.nodes, edges: demoGraph.edges, projects: demoGraph.projects,
      complete: true, warnings: [] }, this.agent.workspaceId, projectRef, includeRelationships);
    let lineage: ProjectRecord[] = [];
    try {
      const projects = await this.listProjects();
      const project = resolveProject(projects, projectRef);
      if (!project) return { nodes: [], edges: [], projects, complete: false, warnings: ["Project reference not found. Use list_projects."] };
      lineage = [project, ...ancestors(projects, project.id)];
      const [nodes, edges] = await Promise.all([
        readWorkspaceRows<GraphNodeRecord>(this.database, "nodes", this.agent.workspaceId, { column: "project_id", values: lineage.map(p => p.id) }),
        readWorkspaceRows<GraphEdgeRecord>(this.database, "edges", this.agent.workspaceId,
          includeRelationships ? undefined : { column: "edge_type", values: ["supersedes"] }),
      ]);
      return scopedSnapshot({ nodes, edges, projects: lineage, complete: true, warnings: [] }, this.agent.workspaceId, projectRef, includeRelationships);
    } catch (error) {
      if (error instanceof Error && /could not be read completely|changed during retrieval/.test(error.message)) {
        return { nodes: [], edges: [], projects: lineage, complete: false, warnings: ["Project coverage is incomplete. Retry before drafting."] };
      }
      throw error;
    }
  }

  async taskBrief(request: BriefRequest) {
    if (!this.agent.scopes.includes("context:read")) throw new Error("context:read is required.");
    const snapshot = await readStableSnapshot(() => this.briefSnapshot(request.project_ref));
    const brief = buildTaskBrief(snapshot, this.agent.workspaceId, request, new Date().toISOString());
    const labels = await this.actorLabels(snapshot.nodes.flatMap(n => [n.created_by_actor_id, n.ratified_by_actor_id]));
    const groups = Object.fromEntries(Object.entries(brief.groups).map(([group, nodes]) => [group, nodes.map(node => ({ ...toolNode(
      node, node.created_by_actor_id ? labels.get(node.created_by_actor_id) ?? null : null,
      node.ratified_by_actor_id ? labels.get(node.ratified_by_actor_id) ?? null : null,
    ), source_project: snapshot.projects?.find(p => p.id === node.project_id) ?? null, inherited: Boolean(node.inherited_from_project_id) }))]));
    const { relationships, reference_edges, reference_notice, ...legacy } = brief;
    return { ...legacy, groups, store: isDemoMode() ? "demo-fixture" : "hosted-supabase", supersession_edges: brief.supersession_edges.map(toolEdge),
      ...(boundedBriefsEnabled() || pilotFeatureEnabled("briefs") ? { relationships: relationships.map(toolEdge), reference_edges: reference_edges.map(edge => ({
        id: edge.id, source_id: edge.source_id, target_id: edge.target_id, edge_type: edge.edge_type, proposed: edge.proposed,
      })), reference_notice } : {}) };
  }

  async deliveredTaskBrief(request: BriefRequest) {
    if (!boundedBriefsEnabled()) requirePilot("briefs");
    const deliveryRequest = withBriefDefaults(request);
    // Ordinary reads never persist baseline manifests. Delta continuation remains
    // explicitly opted into on the experimental server with its own migration.
    const persistBaseline = pilotFeatureEnabled("briefs") && (request.allow_delta === true || Boolean(request.baseline_token));
    return deliverTaskBrief(await this.taskBrief(deliveryRequest), deliveryRequest, {
      workspaceId: this.agent.workspaceId, principalId: `${this.agent.connectionId}:${this.agent.actor.id}`,
      ...(isDemoMode() || !persistBaseline ? {} : { baselineStore: durableBriefBaselineStore(this.database) }),
    });
  }

  async getNode(id: string): Promise<{ node: GraphNodeRecord; actorLabel: string | null; ratifierLabel: string | null }> {
    if (isDemoMode()) {
      const node = demoGraph.nodes.find((record) => record.id === id);
      if (!node) throw new Error(`Node not found: ${id}`);
      return { node, actorLabel: node.actor?.display_name ?? null, ratifierLabel: node.ratified_by_user_id ? "Owner" : null };
    }
    const { data, error } = await this.database.from("nodes").select("*").eq("workspace_id", this.agent.workspaceId).eq("id", id).maybeSingle();
    if (error) throw error;
    if (!data) throw new Error(`Node not found: ${id}`);
    const node = data as GraphNodeRecord;
    const labels = await this.actorLabels([node.created_by_actor_id, node.ratified_by_actor_id]);
    return { node, actorLabel: node.created_by_actor_id ? labels.get(node.created_by_actor_id) ?? null : null, ratifierLabel: node.ratified_by_actor_id ? labels.get(node.ratified_by_actor_id) ?? null : null };
  }

  async expand(id: string) {
    const { node, actorLabel, ratifierLabel } = await this.getNode(id);
    let edges: GraphEdgeRecord[];
    if (isDemoMode()) edges = demoGraph.edges.filter((edge) => edge.source_id === id || edge.target_id === id);
    else {
      const { data, error } = await this.database.from("edges").select("*").eq("workspace_id", this.agent.workspaceId).or(`source_id.eq.${id},target_id.eq.${id}`);
      if (error) throw error;
      edges = (data ?? []) as GraphEdgeRecord[];
    }
    const structured = { node: toolNode(node, actorLabel, ratifierLabel), edges: edges.map(toolEdge) };
    const edgeText = edges.length ? edges.map((edge) => `- ${edge.source_id === id ? "out" : "in"}: ${edge.edge_type} ${edge.source_id === id ? edge.target_id : edge.source_id}${edge.summary ? ` - ${edge.summary}` : ""}`).join("\n") : "Edges: none";
    return { text: `${node.id}: ${node.title}\n${"=".repeat(node.id.length + node.title.length + 2)}\n\n${JSON.stringify(structured.node, null, 2)}\n\n${edgeText}`, structured };
  }

  async capture(type: "Attempt" | "Decision" | "Assumption", input: CaptureInput, supersession?: { id: string; reason: string }) {
    if (!this.agent.scopes.includes("entries:write")) throw new Error("This connection is read only; proposal permission is required.");
    const candidateCapture = input.operation_id !== undefined || input.relationships !== undefined;
    if (candidateCapture) {
      requirePilot("capture");
      if (input.operation_id !== undefined) operationId.parse(input.operation_id);
      if (input.relationships !== undefined) captureRelationship.array().max(20).parse(input.relationships);
      if (isDemoMode()) throw new Error("Retry-safe linked capture requires the isolated persistent test store.");
    }
    const tagged = [...new Set((input.scope_refs ?? []).filter(ref => PROJECT_REF.test(ref)))];
    const projectRef = input.project_ref ?? (tagged.length === 1 ? tagged[0] : undefined);
    if (!projectRef) throw new Error("Choose an owning project with project_ref. Use list_projects to find its reference.");
    const project = resolveProject(await this.listProjects(), projectRef);
    if (!project) throw new Error("Owning project not found. Use list_projects.");
    if (supersession) {
      const target = await this.getNode(supersession.id);
      if (target.node.project_id !== project.id) throw new Error("Supersession must stay within the owning project.");
    }
    const status = type === "Attempt" ? "observation" : type === "Decision" ? "candidate" : "proposed";
    const metadata: Record<string, unknown> = {};
    if (input.evidence_summary) metadata.evidence_summary = input.evidence_summary;
    if (input.source_ref) metadata.source_ref = input.source_ref;
    if (input.outcome) metadata.outcome = input.outcome;
    if (type === "Decision") {
      metadata.chosen_option = input.chosen_option ?? null;
      metadata.rejected_options = input.rejected_options ?? [];
    }
    if (supersession) {
      metadata.proposed_supersedes = supersession.id;
      metadata.superseding_reason = supersession.reason;
    }
    const id = `${PREFIXES[type]}-${randomUUID()}`;
    const edgeId = supersession ? `EDGE-${randomUUID()}` : null;
    if (isDemoMode()) {
      const now = new Date().toISOString();
      const node: GraphNodeRecord = {
        id, workspace_id: this.agent.workspaceId, project_id: project.id, inheritance_mode: "local", type, status, title: input.title,
        summary: input.summary ?? "", body: input.body ?? "", scope_type: input.scope_type ?? null,
        scope_refs: input.scope_refs ?? [], confidence: input.confidence ?? null,
        reversibility: input.reversibility ?? null, consequence: input.consequence ?? null,
        reconsider_conditions: input.reconsider_conditions ?? [], metadata,
        created_by_actor_id: this.agent.actor.id, ratified_by_actor_id: null,
        ratified_by_user_id: null, created_at: now, updated_at: now, ratified_at: null,
      };
      return { node: toolNode(node, this.agent.actor.display_name), proposedEdgeId: edgeId, replayed: false };
    }
    const { data, error } = await this.database.rpc("capture_oauth_node", {
      target_project_id: project.id,
      target_node_id: id,
      target_type: type,
      target_status: status,
      target_title: input.title,
      target_summary: input.summary ?? "",
      target_body: input.body ?? "",
      target_scope_type: input.scope_type ?? null,
      target_scope_refs: input.scope_refs ?? [],
      target_confidence: input.confidence ?? null,
      target_reversibility: input.reversibility ?? null,
      target_consequence: input.consequence ?? null,
      target_reconsider_conditions: input.reconsider_conditions ?? [],
      target_metadata: metadata,
      supersedes_node_id: supersession?.id ?? null,
      supersedes_edge_id: edgeId,
      supersedes_reason: supersession?.reason ?? null,
      ...(candidateCapture ? { target_operation_id: input.operation_id ?? null, target_relations: input.relationships ?? [] } : {}),
    });
    if (error) throw error;
    const result = data as { node: GraphNodeRecord; edge?: GraphEdgeRecord | null; relationships?: GraphEdgeRecord[]; replayed?: boolean };
    return { node: toolNode(result.node, this.agent.actor.display_name), proposedEdgeId: result.edge?.id ?? edgeId,
      ...(candidateCapture ? { relationships: (result.relationships ?? []).map(toolEdge) } : {}),
      replayed: result.replayed ?? false };
  }

  async proposeRelationship(input: RelationshipProposal) {
    requirePilot("capture");
    if (!this.agent.scopes.includes("entries:write")) throw new Error("This connection is read only; proposal permission is required.");
    const value = relationshipProposal.parse(input);
    if (isDemoMode()) throw new Error("Relationship proposals require the isolated persistent test store.");
    const project = resolveProject(await this.listProjects(), value.project_ref);
    if (!project) throw new Error("Owning project not found. Use list_projects.");
    const { data, error } = await this.database.rpc("propose_oauth_relationship", {
      target_project_id: project.id, source_node_id: value.source_id, target_node_id: value.target_id,
      relationship_type: value.edge_type, relationship_rationale: value.rationale,
      relationship_source_ref: value.source_ref, target_operation_id: value.operation_id ?? null,
    });
    if (error) throw error;
    return { edge: toolEdge(data.edge), replayed: data.replayed === true, authority: "proposed; does not approve endpoints or grant inheritance" };
  }

  async checkPlan(task: string, plan: string, projectRef?: string, scope: Pick<BriefRequest, "workflow_ref" | "target_ref" | "context_refs"> = {}) {
    let relevant;
    let coverage: { complete: boolean; scope_questions: unknown[]; warnings: string[]; fingerprint: string } | undefined;
    if (pilotFeatureEnabled("plan-scope")) {
      if (!projectRef) throw new Error("Candidate plan checking requires project_ref.");
      if (!this.agent.scopes.includes("context:read")) throw new Error("context:read is required.");
      const snapshot = await readStableSnapshot(() => this.briefSnapshot(projectRef));
      const brief = buildTaskBrief(snapshot, this.agent.workspaceId, { project_ref: projectRef, task, ...scope }, new Date().toISOString());
      const nodes = Object.values(brief.groups).flat();
      const labels = await this.actorLabels(nodes.flatMap(n => [n.created_by_actor_id, n.ratified_by_actor_id]));
      const ranked = rankHostedNodes(nodes, `${task}\n${plan}`, nodes.length, labels);
      const scored = new Map(ranked.map(match => [match.node.id, match.score]));
      // Every applicable active Decision participates, including those with no keyword overlap.
      relevant = nodes.filter(n => n.status === "active" || n.status === "accepted" || scored.has(n.id)).map(node => ({
        node, score: scored.get(node.id) ?? 0,
        toolNode: toolNode(node, node.created_by_actor_id ? labels.get(node.created_by_actor_id) ?? null : null,
          node.ratified_by_actor_id ? labels.get(node.ratified_by_actor_id) ?? null : null),
      }));
      coverage = { complete: brief.complete, scope_questions: brief.scope_questions, warnings: brief.warnings, fingerprint: brief.fingerprint };
    } else relevant = await this.search(`${task}\n${plan}`, 12, projectRef);
    const combined = `${task}\n${plan}`.toLowerCase();
    const conflicts = relevant.filter(({ node }) => node.type === "Decision" && node.status === "active").map((match) => {
      const rejected = Array.isArray(match.node.metadata.rejected_options) ? match.node.metadata.rejected_options.map(String).filter(Boolean) : [];
      const matched = rejected.filter((option) => combined.includes(option.toLowerCase()));
      return matched.length ? { node: match.toolNode, score: match.score, matched_rejected_options: matched } : null;
    }).filter((item): item is NonNullable<typeof item> => item !== null);
    const structured = { conflicts, relevant: relevant.map((match) => ({ node: match.toolNode, score: match.score })), advisory: true,
      ...(coverage ? { ...coverage, method: "literal rejected-option match; agent must assess semantic conflicts" } : {}) };
    const text = coverage && (!coverage.complete || coverage.scope_questions.length)
      ? "Plan check incomplete: resolve coverage and scope questions before interpreting conflicts."
      : conflicts.length
      ? `Plan check: advisory conflicts found.\n\nThis plan appears to contradict active Neurofilament decisions.\nRevise the plan or propose a superseding decision with new evidence.\n\n${conflicts.map((item) => `- ${item.node.id}: ${item.node.title}\n  Matched rejected options: ${item.matched_rejected_options.join(", ")}`).join("\n")}`
      : `Plan check: no direct conflicts found.\n\n${relevant.length ? "Relevant guidance:\n" + relevant.map((item) => compact(item.node, item.score)).join("\n\n") : "No relevant Neurofilament decisions found."}`;
    return { text, structured };
  }
}
