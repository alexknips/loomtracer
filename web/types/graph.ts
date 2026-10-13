export type NodeKind =
  | "Decision"
  | "Option"
  | "Evidence"
  | "Assumption"
  | "Attempt"
  | "Artifact"
  | "Scope";

export type NodeStatus =
  | "observation"
  | "candidate"
  | "active"
  | "superseded"
  | "rejected"
  | "invalidated"
  | "archived"
  | "proposed"
  | "accepted"
  | "expired";

export interface ActorRecord {
  id: string;
  display_name: string;
  kind: "human" | "agent" | "system" | "import";
  platform: string | null;
}

export interface GraphNodeRecord {
  id: string;
  workspace_id: string;
  type: NodeKind;
  status: NodeStatus;
  title: string;
  summary: string;
  body: string;
  scope_type: string | null;
  scope_refs: string[];
  confidence: string | null;
  reversibility: string | null;
  consequence: string | null;
  reconsider_conditions: string[];
  metadata: Record<string, unknown>;
  created_by_actor_id: string | null;
  ratified_by_actor_id: string | null;
  ratified_by_user_id: string | null;
  created_at: string;
  updated_at: string;
  ratified_at: string | null;
  actor?: ActorRecord | null;
  project_id?: string | null;
  inheritance_mode?: "local" | "descendants";
  inherited_from_project_id?: string;
}

export interface ProjectRecord {
  id: string;
  workspace_id: string;
  ref: string;
  name: string;
  description: string;
  parent_id: string | null;
  created_at: string;
  updated_at: string;
}

export interface GraphEdgeRecord {
  id: string;
  workspace_id: string;
  source_id: string;
  edge_type: string;
  target_id: string;
  summary: string;
  metadata: Record<string, unknown>;
  created_at: string;
}

export interface AuditEventRecord {
  id: string;
  action: string;
  actor_label: string;
  reason: string | null;
  object_id?: string;
  created_at: string;
}

export interface AgentTokenRecord {
  id: string;
  name: string;
  token_prefix: string;
  scopes: string[];
  expires_at: string;
  last_used_at: string | null;
  revoked_at: string | null;
  created_at: string;
  actor?: ActorRecord | null;
}

export interface GraphPayload {
  root_id: string | null;
  nodes: GraphNodeRecord[];
  edges: GraphEdgeRecord[];
  audits: Record<string, AuditEventRecord[]>;
  truncated: boolean;
  view?: "all" | "neighborhood";
  depth?: number;
  total_nodes?: number;
  total_edges?: number;
  projects?: ProjectRecord[];
  project_filter?: string;
  inherited_nodes?: GraphNodeRecord[];
  include_inherited?: boolean;
  cross_project_edges?: GraphEdgeRecord[];
  related_nodes?: GraphNodeRecord[];
  warnings?: string[];
}

export interface ReviewRecord extends GraphNodeRecord {
  proposed_supersedes?: GraphNodeRecord | null;
}
