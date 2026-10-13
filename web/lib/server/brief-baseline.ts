import { recordHash, selectedBriefScope, type BriefRequest } from "./task-brief";

export interface BriefBaselineManifest {
  version: 1;
  scope_key: string;
  fingerprint: string;
  nodes: Record<string, { hash: string; group: string }>;
  relationships: Record<string, string>;
  metadata_hash: string;
}

export interface StoredBriefBaseline {
  token: string;
  expires_at: string;
  manifest: BriefBaselineManifest;
}

/** Implement with authenticated durable storage, never a process-local cache. */
export interface BriefBaselineStore {
  get(token: string, projectRef: string, scopeKey: string): Promise<StoredBriefBaseline | null>;
  save(projectRef: string, scopeKey: string, manifest: BriefBaselineManifest): Promise<{ token: string; expires_at: string }>;
}

export function briefScopeKey(workspaceId: string, principalId: string, request: BriefRequest) {
  return recordHash({
    contract: "brief-delivery-v1", workspaceId, principalId, project: request.project_ref,
    scope: selectedBriefScope(request), task: request.task,
    delivery: request.delivery ?? "full", max_response_bytes: request.max_response_bytes ?? 100000,
    max_linked_nodes: request.max_linked_nodes ?? 8,
    // Preserve incumbent default keys while binding every opt-in selection limit.
    ...(request.max_task_evidence_nodes ? { max_task_evidence_nodes: request.max_task_evidence_nodes } : {}),
  });
}

type ManifestPayload = {
  fingerprint: string;
  groups: Record<string, Array<{ id: string }>>;
  relationships: Array<{ id: string }>;
  [key: string]: unknown;
};

/** Only hashes/IDs are retained. Bodies and evidence stay in the graph. */
export function makeBriefManifest(payload: ManifestPayload, scopeKey: string): BriefBaselineManifest {
  const nodes = Object.fromEntries(Object.entries(payload.groups).flatMap(([group, records]) => records.map(node => [node.id, { hash: recordHash(node), group }])));
  const relationships = Object.fromEntries(payload.relationships.map(edge => [edge.id, recordHash(edge)]));
  const metadata = { ...payload };
  for (const key of ["groups", "relationships", "supersession_edges", "checked_at", "fingerprint", "changed", "record_versions", "baseline", "delivery"]) delete metadata[key];
  return { version: 1, scope_key: scopeKey, fingerprint: payload.fingerprint, nodes, relationships, metadata_hash: recordHash(metadata) };
}

export function diffBriefManifests(previous: BriefBaselineManifest, current: BriefBaselineManifest) {
  if (previous.version !== current.version || previous.scope_key !== current.scope_key) throw new Error("Brief baseline scope mismatch.");
  return {
    changed_node_ids: Object.keys(current.nodes).filter(id => previous.nodes[id]?.hash !== current.nodes[id].hash || previous.nodes[id]?.group !== current.nodes[id].group),
    removed_node_ids: Object.keys(previous.nodes).filter(id => !current.nodes[id]),
    changed_relationship_ids: Object.keys(current.relationships).filter(id => previous.relationships[id] !== current.relationships[id]),
    removed_relationship_ids: Object.keys(previous.relationships).filter(id => !current.relationships[id]),
    metadata_changed: previous.metadata_hash !== current.metadata_hash,
  };
}

export function usableBriefBaseline(baseline: StoredBriefBaseline | null, token: string, scopeKey: string, now: number): baseline is StoredBriefBaseline {
  if (!baseline || baseline.token !== token || baseline.manifest?.version !== 1
    || baseline.manifest.scope_key !== scopeKey || !(Date.parse(baseline.expires_at) > now)) return false;
  const manifest = baseline.manifest;
  const hash = (value: unknown) => typeof value === "string" && /^[a-f0-9]{64}$/.test(value);
  const record = (value: unknown): value is Record<string, unknown> => Boolean(value && typeof value === "object" && !Array.isArray(value));
  return hash(manifest.fingerprint) && hash(manifest.metadata_hash) && record(manifest.nodes) && record(manifest.relationships)
    && Object.values(manifest.nodes).every(value => record(value) && hash(value.hash) && typeof value.group === "string")
    && Object.values(manifest.relationships).every(hash);
}
