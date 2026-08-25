import { randomUUID } from "node:crypto";
import { createAdminClient } from "@/lib/supabase/admin";
import { demoGraph } from "@/lib/demo";
import { isDemoMode } from "@/lib/env";
import type { PatContext } from "@/lib/server/tokens";
import type { GraphEdgeRecord, GraphNodeRecord } from "@/types/graph";

type ToolNode = Omit<GraphNodeRecord, "workspace_id" | "created_by_actor_id" | "ratified_by_actor_id" | "ratified_by_user_id" | "actor"> & {
  created_by: string | null;
  ratified_by: string | null;
};

export interface CaptureInput {
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
}

const STATUS_WEIGHTS: Record<string, number> = { active: 8, accepted: 8, candidate: 5, observation: 4, proposed: 4, superseded: 2, rejected: 2, invalidated: 1, archived: 1 };
const TYPE_WEIGHTS: Record<string, number> = { Decision: 4, Attempt: 3, Assumption: 3, Evidence: 2, Option: 1, Artifact: 1, Actor: 1, Scope: 1 };
const STOPWORDS = new Set(["a", "an", "and", "are", "as", "be", "by", "for", "from", "in", "into", "is", "it", "of", "on", "or", "that", "the", "this", "to", "use", "with"]);
const PREFIXES: Record<string, string> = { Decision: "D", Option: "O", Evidence: "E", Assumption: "A", Attempt: "T", Actor: "ACT", Artifact: "ART", Scope: "S" };

function tokenize(text: string): string[] {
  return (text.toLowerCase().match(/[a-z0-9_][a-z0-9_.-]*/g) ?? []).filter((token) => token.length >= 3 && !STOPWORDS.has(token));
}

function corpus(node: GraphNodeRecord): string {
  return [node.id, node.type, node.status, node.title, node.summary, node.body, node.scope_type ?? "", node.scope_refs.join(" "), node.confidence ?? "", node.reversibility ?? "", node.consequence ?? "", node.reconsider_conditions.join(" "), JSON.stringify(node.metadata)].join("\n").toLowerCase();
}

export function rankHostedNodes(nodes: GraphNodeRecord[], query: string, limit = 10): Array<{ node: GraphNodeRecord; score: number }> {
  const tokens = tokenize(query);
  if (!tokens.length) return [];
  return nodes.map((node) => {
    const text = corpus(node);
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
  const result = { ...node } as Partial<GraphNodeRecord> & Record<string, unknown>;
  delete result.workspace_id;
  delete result.created_by_actor_id;
  delete result.ratified_by_actor_id;
  delete result.ratified_by_user_id;
  delete result.actor;
  return { ...result, created_by: actorLabel, ratified_by: ratifierLabel ?? node.ratified_by_user_id } as ToolNode;
}

function toolEdge(edge: GraphEdgeRecord) {
  const result = { ...edge } as Partial<GraphEdgeRecord>;
  delete result.workspace_id;
  return result as Omit<GraphEdgeRecord, "workspace_id">;
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
  constructor(private readonly pat: PatContext) {}

  private get admin() { return createAdminClient(); }

  async allNodes(): Promise<GraphNodeRecord[]> {
    if (isDemoMode()) return demoGraph.nodes;
    const { data, error } = await this.admin.from("nodes").select("*").eq("workspace_id", this.pat.workspaceId).limit(5000);
    if (error) throw error;
    return (data ?? []) as GraphNodeRecord[];
  }

  async actorLabels(ids: Array<string | null>): Promise<Map<string, string>> {
    const actorIds = [...new Set(ids.filter((value): value is string => Boolean(value)))];
    if (!actorIds.length) return new Map();
    if (isDemoMode()) return new Map(demoGraph.nodes.filter((node) => node.actor).map((node) => [node.actor!.id, node.actor!.display_name]));
    const { data, error } = await this.admin.from("actors").select("id,display_name").eq("workspace_id", this.pat.workspaceId).in("id", actorIds);
    if (error) throw error;
    return new Map((data ?? []).map((actor) => [actor.id, actor.display_name]));
  }

  async search(query: string, limit = 10) {
    const ranked = rankHostedNodes(await this.allNodes(), query, limit);
    const labels = await this.actorLabels(ranked.flatMap(({ node }) => [node.created_by_actor_id, node.ratified_by_actor_id]));
    return ranked.map(({ node, score }) => ({ node, score, toolNode: toolNode(
      node,
      node.created_by_actor_id ? labels.get(node.created_by_actor_id) ?? null : null,
      node.ratified_by_actor_id ? labels.get(node.ratified_by_actor_id) ?? null : null,
    ) }));
  }

  async relevantContext(task: string, files: string[], technologies: string[], proposedPlan: string, tokenBudget: number) {
    const matches = await this.search([task, proposedPlan, files.join(" "), technologies.join(" ")].join(" "), 12);
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

  async getNode(id: string): Promise<{ node: GraphNodeRecord; actorLabel: string | null; ratifierLabel: string | null }> {
    if (isDemoMode()) {
      const node = demoGraph.nodes.find((record) => record.id === id);
      if (!node) throw new Error(`Node not found: ${id}`);
      return { node, actorLabel: node.actor?.display_name ?? null, ratifierLabel: node.ratified_by_user_id ? "Owner" : null };
    }
    const { data, error } = await this.admin.from("nodes").select("*").eq("workspace_id", this.pat.workspaceId).eq("id", id).maybeSingle();
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
      const { data, error } = await this.admin.from("edges").select("*").eq("workspace_id", this.pat.workspaceId).or(`source_id.eq.${id},target_id.eq.${id}`);
      if (error) throw error;
      edges = (data ?? []) as GraphEdgeRecord[];
    }
    const structured = { node: toolNode(node, actorLabel, ratifierLabel), edges: edges.map(toolEdge) };
    const edgeText = edges.length ? edges.map((edge) => `- ${edge.source_id === id ? "out" : "in"}: ${edge.edge_type} ${edge.source_id === id ? edge.target_id : edge.source_id}${edge.summary ? ` - ${edge.summary}` : ""}`).join("\n") : "Edges: none";
    return { text: `${node.id}: ${node.title}\n${"=".repeat(node.id.length + node.title.length + 2)}\n\n${JSON.stringify(structured.node, null, 2)}\n\n${edgeText}`, structured };
  }

  async capture(type: "Attempt" | "Decision" | "Assumption", input: CaptureInput, supersession?: { id: string; reason: string }) {
    if (!this.pat.scopes.includes("entries:write")) throw new Error("This token has context:read scope only; entries:write is required.");
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
        id, workspace_id: this.pat.workspaceId, type, status, title: input.title,
        summary: input.summary ?? "", body: input.body ?? "", scope_type: input.scope_type ?? null,
        scope_refs: input.scope_refs ?? [], confidence: input.confidence ?? null,
        reversibility: input.reversibility ?? null, consequence: input.consequence ?? null,
        reconsider_conditions: input.reconsider_conditions ?? [], metadata,
        created_by_actor_id: this.pat.actor.id, ratified_by_actor_id: null,
        ratified_by_user_id: null, created_at: now, updated_at: now, ratified_at: null,
      };
      return { node: toolNode(node, this.pat.actor.display_name), proposedEdgeId: edgeId };
    }
    const { data, error } = await this.admin.rpc("capture_agent_node", {
      target_workspace_id: this.pat.workspaceId,
      target_actor_id: this.pat.actor.id,
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
    });
    if (error) throw error;
    const node = (data as { node: GraphNodeRecord }).node;
    return { node: toolNode(node, this.pat.actor.display_name), proposedEdgeId: edgeId };
  }

  async checkPlan(task: string, plan: string) {
    const relevant = await this.search(`${task}\n${plan}`, 12);
    const combined = `${task}\n${plan}`.toLowerCase();
    const conflicts = relevant.filter(({ node }) => node.type === "Decision" && node.status === "active").map((match) => {
      const rejected = Array.isArray(match.node.metadata.rejected_options) ? match.node.metadata.rejected_options.map(String).filter(Boolean) : [];
      const matched = rejected.filter((option) => combined.includes(option.toLowerCase()));
      return matched.length ? { node: match.toolNode, score: match.score, matched_rejected_options: matched } : null;
    }).filter((item): item is NonNullable<typeof item> => item !== null);
    const structured = { conflicts, relevant: relevant.map((match) => ({ node: match.toolNode, score: match.score })), advisory: true };
    const text = conflicts.length
      ? `Plan check: advisory conflicts found.\n\nThis plan appears to contradict active Neurofilament decisions.\nRevise the plan or propose a superseding decision with new evidence.\n\n${conflicts.map((item) => `- ${item.node.id}: ${item.node.title}\n  Matched rejected options: ${item.matched_rejected_options.join(", ")}`).join("\n")}`
      : `Plan check: no direct conflicts found.\n\n${relevant.length ? "Relevant guidance:\n" + relevant.map((item) => compact(item.node, item.score)).join("\n\n") : "No relevant Neurofilament decisions found."}`;
    return { text, structured };
  }
}
