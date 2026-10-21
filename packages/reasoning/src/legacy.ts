import { createHash } from 'node:crypto';
import { validateBundle } from './domain';
import type { Actor, Connection, Entry, Json, ReasoningBundle, SourceMessage } from './types';

export type LegacyRow = { id: string; workspace_id: string; [key: string]: Json };
export type LegacySnapshot = {
  snapshotId: string; createdAt: string;
  tables: { nodes: LegacyRow[]; edges: LegacyRow[]; actors: LegacyRow[]; projects: LegacyRow[]; audit_events: LegacyRow[]; [table: string]: Json[] };
  schema?: Json;
};
export type MigrationMap = { table: string; workspaceId: string; legacyId: string; datasetId: string; sourceId: string; entryId?: string; connectionId?: string; sha256: string };
export type ProjectAssignment = { workspaceId: string; nodeId: string; projectId: string; reason: string };
export type ConversionResult = {
  bundles: ReasoningBundle[];
  report: {
    snapshotId: string; snapshotSha256: string; ready: boolean;
    inputCounts: Record<string, number>; converted: { nodes: number; edges: number; audits: number };
    mappings: MigrationMap[]; concerns: { kind: string; workspaceId: string; recordId: string; message: string }[];
    legacyApprovals: { datasetId: string; entryId: string; receiptId: string; legacyStatus: string; eligibleForCurrent: boolean }[];
    retainedUnassignedAudits: LegacyRow[]; unassignedNodes: LegacyRow[];
    explicitProjectAssignments: ProjectAssignment[];
  };
};

export function canonicalJson(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(',')}]`;
  if (value && typeof value === 'object') return `{${Object.keys(value).sort().map(key => `${JSON.stringify(key)}:${canonicalJson((value as Record<string, unknown>)[key])}`).join(',')}}`;
  return JSON.stringify(value);
}
const hash = (value: unknown) => createHash('sha256').update(canonicalJson(value)).digest('hex');
const text = (row: LegacyRow, key: string) => typeof row[key] === 'string' ? row[key] as string : null;
const timestamp = (value: string | null) => value && /(?:Z|[+-]\d{2}:\d{2})$/.test(value) && Number.isFinite(Date.parse(value)) ? value : null;
const key = (workspaceId: string, id: string) => `${workspaceId}\u0000${id}`;
const encoded = (value: string) => encodeURIComponent(value);
const datasetId = (workspaceId: string, projectId: string) => `legacy:${encoded(workspaceId)}:${encoded(projectId)}`;
const recordId = (table: string, id: string) => `legacy:${table}:${encoded(id)}`;

/** Deterministic preservation conversion, not semantic reconstruction. Never contacts the database. */
export function convertLegacySnapshot(snapshot: LegacySnapshot, options: { projectAssignments?: ProjectAssignment[] } = {}): ConversionResult {
  if (!snapshot.snapshotId || !timestamp(snapshot.createdAt)) throw new Error('Snapshot requires an identity and timezone-qualified creation time');
  for (const name of ['nodes', 'edges', 'actors', 'projects', 'audit_events'] as const) {
    if (!Array.isArray(snapshot.tables[name])) throw new Error(`Missing complete snapshot table: ${name}`);
    const seen = new Set<string>();
    for (const row of snapshot.tables[name]) {
      if (!row.id || !row.workspace_id) throw new Error(`${name} row needs id and workspace_id`);
      const identity = key(row.workspace_id, row.id);
      if (seen.has(identity)) throw new Error(`Duplicate snapshot identity ${name}:${row.workspace_id}:${row.id}`);
      seen.add(identity);
    }
  }
  const result: ConversionResult = { bundles: [], report: { snapshotId: snapshot.snapshotId, snapshotSha256: hash(snapshot), ready: true, inputCounts: Object.fromEntries(Object.entries(snapshot.tables).map(([name, rows]) => [name, rows.length])), converted: { nodes: 0, edges: 0, audits: 0 }, mappings: [], concerns: [], legacyApprovals: [], retainedUnassignedAudits: [], unassignedNodes: [], explicitProjectAssignments: options.projectAssignments ?? [] } };
  const report = result.report;
  const projectMap = new Map(snapshot.tables.projects.map(project => [key(project.workspace_id, project.id), project]));
  const nodeMap = new Map(snapshot.tables.nodes.map(node => [key(node.workspace_id, node.id), node]));
  const edgeMap = new Map(snapshot.tables.edges.map(edge => [key(edge.workspace_id, edge.id), edge]));
  const assignments = new Map<string, string>();
  for (const assignment of options.projectAssignments ?? []) {
    const identity = key(assignment.workspaceId, assignment.nodeId);
    if (assignments.has(identity) || !assignment.reason.trim() || !nodeMap.has(identity) || !projectMap.has(key(assignment.workspaceId, assignment.projectId))) throw new Error(`Invalid explicit project assignment: ${assignment.nodeId}`);
    if (text(nodeMap.get(identity)!, 'project_id')) throw new Error(`Project assignment cannot overwrite existing ownership: ${assignment.nodeId}`);
    assignments.set(identity, assignment.projectId);
  }
  const projectOf = (node: LegacyRow) => assignments.get(key(node.workspace_id, node.id)) ?? text(node, 'project_id');
  const ownedNodes = new Map<string, LegacyRow[]>();
  const problem = (kind: string, row: LegacyRow, message: string, blocking = true) => { report.concerns.push({ kind, workspaceId: row.workspace_id, recordId: row.id, message }); if (blocking) report.ready = false; };
  for (const node of snapshot.tables.nodes) {
    const projectId = projectOf(node);
    if (!projectId || !projectMap.has(key(node.workspace_id, projectId))) { report.unassignedNodes.push(node); problem('unassigned_node', node, 'Project ownership is absent or unresolved. Preserve the original row and resolve ownership before migration.'); continue; }
    const group = key(node.workspace_id, projectId);
    ownedNodes.set(group, [...(ownedNodes.get(group) ?? []), node]);
  }
  const assignedAudits = new Set<string>();
  for (const project of [...snapshot.tables.projects].sort((a, b) => key(a.workspace_id, a.id).localeCompare(key(b.workspace_id, b.id)))) {
    const nodes = [...(ownedNodes.get(key(project.workspace_id, project.id)) ?? [])].sort((a, b) => a.id.localeCompare(b.id));
    const nodeIds = new Set(nodes.map(node => node.id));
    const edges = snapshot.tables.edges.filter(edge => edge.workspace_id === project.workspace_id && nodeIds.has(text(edge, 'source_id') ?? '')).sort((a, b) => a.id.localeCompare(b.id));
    const edgeIds = new Set(edges.map(edge => edge.id));
    const audits = snapshot.tables.audit_events.filter(audit => {
      if (audit.workspace_id !== project.workspace_id) return false;
      const objectId = text(audit, 'object_id'), type = text(audit, 'object_type');
      return ((type === 'node' || type === 'nodes') && !!objectId && nodeIds.has(objectId)) || ((type === 'edge' || type === 'edges') && !!objectId && edgeIds.has(objectId)) || ((type === 'project' || type === 'projects') && objectId === project.id);
    }).sort((a, b) => a.id.localeCompare(b.id));
    audits.forEach(audit => assignedAudits.add(key(audit.workspace_id, audit.id)));
    const id = datasetId(project.workspace_id, project.id), contextId = 'legacy:context', importerId = 'import:legacy-converter', unknownId = 'unknown:legacy-origin';
    const actors: Actor[] = snapshot.tables.actors.filter(actor => actor.workspace_id === project.workspace_id).sort((a, b) => a.id.localeCompare(b.id)).map(actor => ({ id: actor.id, name: text(actor, 'display_name') ?? actor.id, kind: ['human', 'agent', 'system', 'import'].includes(text(actor, 'kind') ?? '') ? text(actor, 'kind') as Actor['kind'] : 'unknown' }));
    actors.push({ id: importerId, name: 'Legacy preservation converter', kind: 'import' }, { id: unknownId, name: 'Original author not established', kind: 'unknown' });
    const actorIds = new Set(actors.map(actor => actor.id));
    const bundle: ReasoningBundle = { schemaVersion: '0.3.0', id, title: text(project, 'name') ?? project.id, namespace: { datasetId: id, workspaceId: project.workspace_id, projectId: project.id }, actors, contexts: [{ id: contextId, title: 'Preserved legacy records; not a conversation transcript', kind: 'legacy_record' }], sources: [], attachments: [], manifest: { threadId: id, frozenAt: snapshot.createdAt, cutoffDescription: `Legacy snapshot ${snapshot.snapshotId}. Source order is deterministic snapshot order, not conversation chronology.`, segments: [], exclusions: ['Credential secrets and authentication tokens are not conversation sources.'] }, entries: [], connections: [], runs: [], reviewNotes: [], authorityGrants: [], commitmentReceipts: [], externalEntryRefs: [] };
    const addSource = (table: string, row: LegacyRow, content?: string): SourceMessage => {
      const sourceId = `source:${recordId(table, row.id)}`;
      const originalRecorder = text(row, 'created_by_actor_id');
      const source: SourceMessage = { id: sourceId, originalId: `${row.workspace_id}:${table}:${row.id}`, actorId: importerId, role: table === 'audit_events' ? 'audit' : 'record', content: content ?? canonicalJson(row), occurredAt: null, loggedAt: timestamp(text(row, 'created_at')), order: bundle.sources.length + 1, sourceRef: `snapshot:${encoded(snapshot.snapshotId)}/${table}/${encoded(row.id)}`, attachmentIds: [], active: true, provenance: { kind: table === 'audit_events' ? 'legacy_audit' : 'legacy_record', originalRecord: { table, id: row.id, workspaceId: row.workspace_id }, recordedBy: originalRecorder && actorIds.has(originalRecorder) ? originalRecorder : null, recordedAt: timestamp(text(row, 'created_at')), payload: row } };
      bundle.sources.push(source);
      bundle.manifest.segments.push({ ref: source.sourceRef, sha256: hash(row), selectedMessageIds: [sourceId] });
      report.mappings.push({ table, workspaceId: row.workspace_id, legacyId: row.id, datasetId: id, sourceId, sha256: hash(row) });
      return source;
    };
    const span = (source: SourceMessage) => ({ messageId: source.id, start: 0, end: source.content.length });
    const provenance = (source: SourceMessage): Entry['provenance'] => ({ expressedBy: null, recordedBy: importerId, occurredAt: null, recordedAt: snapshot.createdAt, basis: 'direct', sourceSpans: [span(source)] });
    addSource('projects', project);
    for (const assignment of (options.projectAssignments ?? []).filter(item => item.workspaceId === project.workspace_id && item.projectId === project.id)) addSource('migration_project_assignments', { id: assignment.nodeId, workspace_id: assignment.workspaceId, project_id: assignment.projectId, reason: assignment.reason });
    snapshot.tables.actors.filter(actor => actor.workspace_id === project.workspace_id).sort((a, b) => a.id.localeCompare(b.id)).forEach(actor => addSource('actors', actor));
    for (const node of nodes) {
      const readableFields = ['title', 'summary', 'body', 'type', 'status', 'scope_type', 'scope_refs', 'inheritance_mode', 'confidence', 'reversibility', 'consequence', 'reconsider_conditions', 'metadata'];
      const source = addSource('nodes', node, readableFields.filter(field => field in node).map(field => `${field}:\n${typeof node[field] === 'string' ? node[field] : canonicalJson(node[field])}`).join('\n\n'));
      const entryId = `${recordId('nodes', node.id)}:v1`;
      bundle.entries.push({ id: entryId, logicalId: recordId('nodes', node.id), version: 1, contextId, summary: text(node, 'title') ?? node.id, labels: [`legacy-type:${text(node, 'type') ?? 'unknown'}`, `legacy-status:${text(node, 'status') ?? 'unknown'}`], provenance: provenance(source) });
      report.mappings[report.mappings.length - 1].entryId = entryId;
      report.converted.nodes++;
      const ratifier = text(node, 'ratified_by_actor_id'), approvedAt = timestamp(text(node, 'ratified_at'));
      const active = text(node, 'type') === 'Decision' && text(node, 'status') === 'active';
      const verifiedHuman = ratifier && approvedAt && actorIds.has(ratifier) && actors.find(actor => actor.id === ratifier)?.kind === 'human' && text(node, 'ratified_by_user_id');
      if (active && !verifiedHuman) {
        const receiptId = `receipt:${recordId('nodes', node.id)}:legacy-state`;
        bundle.commitmentReceipts.push({ id: receiptId, action: 'adopt', entryId, priorReceiptId: null, actorId: unknownId, grantId: null, recordedAt: snapshot.createdAt, rationale: 'Preserved recorded legacy active state; original ratifying identity is not established. This receipt records migration, not a new approval. Original ratification fields remain in the source.', connectionId: null, basis: 'legacy_state', sourceSpans: [span(source)] });
        report.legacyApprovals.push({ datasetId: id, entryId, receiptId, legacyStatus: 'active', eligibleForCurrent: true });
        problem('legacy_authority_identity_unresolved', node, 'Current active legacy status is preserved as legacy_state, with unknown ratifying identity. The receipt time is migration time; original attribution and dates remain unchanged in the source.', false);
      } else if (text(node, 'type') === 'Decision' && ratifier && approvedAt && actorIds.has(ratifier)) {
        const receiptId = `receipt:${recordId('nodes', node.id)}:approval`;
        bundle.commitmentReceipts.push({ id: receiptId, action: 'adopt', entryId, priorReceiptId: null, actorId: ratifier, grantId: null, recordedAt: approvedAt, rationale: '', connectionId: null, basis: 'legacy_approval', sourceSpans: [span(source)] });
        report.legacyApprovals.push({ datasetId: id, entryId, receiptId, legacyStatus: text(node, 'status') ?? 'unknown', eligibleForCurrent: active });
      } else if (text(node, 'type') === 'Decision' && text(node, 'status') === 'superseded') problem('historical_approval_not_established', node, 'Inactive historical status is preserved, but the original ratifier or ratification time is incomplete. No historical approval was invented.', false);
    }
    for (const edge of edges) {
      const source = addSource('edges', edge);
      const targetId = text(edge, 'target_id'), sourceId = text(edge, 'source_id'), relation = text(edge, 'edge_type');
      const target = targetId ? nodeMap.get(key(edge.workspace_id, targetId)) : undefined;
      if (!sourceId || !target || !relation || !/^[a-zA-Z0-9_.-]+$/.test(relation)) { problem('invalid_edge', edge, 'Edge endpoint or exact relation is missing; original edge source is retained.'); continue; }
      let targetEntryId = `${recordId('nodes', target.id)}:v1`;
      const targetProject = projectOf(target);
      const crossProject = targetProject !== project.id;
      if (crossProject) {
        if (!targetProject || !projectMap.has(key(target.workspace_id, targetProject))) { problem('unassigned_edge_target', edge, 'Target project is unresolved. Original edge source remains preserved.'); continue; }
        const externalId = `external:${encoded(targetProject)}:${encoded(target.id)}`;
        if (!bundle.externalEntryRefs.some(ref => ref.id === externalId)) bundle.externalEntryRefs.push({ id: externalId, namespace: { datasetId: datasetId(target.workspace_id, targetProject), workspaceId: target.workspace_id, projectId: targetProject }, entryId: targetEntryId, originalId: target.id });
        targetEntryId = externalId;
        problem('cross_project_edge', edge, 'Preserved once in the source project with an external target reference; no foreign entry copied.', false);
      }
      const connectionId = `${recordId('edges', edge.id)}:v1`;
      // Only legacy supports has precisely matching native premise -> conclusion semantics.
      const native = relation === 'supports' && !crossProject;
      const connection: Connection = { id: connectionId, logicalId: recordId('edges', edge.id), version: 1, contextId, relation: native ? 'supports' : `legacy:${relation}`, participants: [{ entryId: `${recordId('nodes', sourceId)}:v1`, role: native ? 'premise' : 'source' }, { entryId: targetEntryId, role: native ? 'conclusion' : 'target' }], provenance: provenance(source) };
      bundle.connections.push(connection); report.mappings[report.mappings.length - 1].connectionId = connectionId; report.converted.edges++;
    }
    for (const audit of audits) { addSource('audit_events', audit); report.converted.audits++; }
    bundle.runs.push({ id: 'legacy:preservation:v1', parentId: null, createdAt: snapshot.createdAt, recordedBy: importerId, sourceCutoff: bundle.sources.length, sourceMessageIds: bundle.sources.map(source => source.id), addedEntryIds: bundle.entries.map(entry => entry.id), addedConnectionIds: bundle.connections.map(connection => connection.id), replacements: [], uncertainties: ['Original authors and contribution occurrence times are not established by database creator or creation time.', 'Legacy status and approval history are preserved facts, not new approvals. Portable receipts require server verification.', 'This is preservation conversion. Missing rationale, causal links, and criteria require separately reviewed extraction.'], notes: 'Lossless legacy preservation; database record order does not assert conversation order. Semantic enrichment remains a later extraction revision.', reviewResolutions: [] });
    const validated = validateBundle(bundle);
    if (!validated.ok) throw new Error(`Converted dataset ${id} failed validation: ${validated.errors.join('; ')}`);
    result.bundles.push(validated.bundle!);
  }
  for (const edge of snapshot.tables.edges) if (!report.mappings.some(mapping => mapping.table === 'edges' && mapping.workspaceId === edge.workspace_id && mapping.legacyId === edge.id)) problem('unassigned_edge_source', edge, 'Source node ownership is unresolved. Edge remains in the original snapshot; migration is blocked.');
  report.retainedUnassignedAudits = snapshot.tables.audit_events.filter(audit => !assignedAudits.has(key(audit.workspace_id, audit.id)));
  for (const audit of report.retainedUnassignedAudits) {
    const objectId = text(audit, 'object_id'), type = text(audit, 'object_type');
    const deleted = objectId && ((type === 'node' && !nodeMap.has(key(audit.workspace_id, objectId))) || (type === 'edge' && !edgeMap.has(key(audit.workspace_id, objectId))));
    problem('unassigned_audit', audit, deleted ? 'Audit targets a no-longer-present record. Original audit is retained in the report and needs explicit ownership resolution.' : 'Workspace or unassigned audit is retained in the report; it was not silently assigned to every project.', false);
  }
  return result;
}
