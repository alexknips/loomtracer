import { describe, expect, it } from 'vitest';
import { makeDemoBundle as oldDemo } from './fixture';
import { buildEvaluationPacket, entryText, getView, mergeBundles, relatedConnections, upgradeBundle, validateBundle } from '../src/domain';
import { buildGraphModel, projectGraphModel, relationLinks } from '../src/graph-model';
import { convertLegacySnapshot, type LegacySnapshot } from '../src/legacy';
import type { CommitmentReceipt } from '../src/types';

const demo = () => upgradeBundle(oldDemo());
const timestamp = '2026-10-12T12:00:00Z';
function snapshot(): LegacySnapshot {
  return {
    snapshotId: 'snapshot-1', createdAt: timestamp,
    tables: {
      workspaces: [{ id: 'w1', name: 'One' }], workspace_members: [{ workspace_id: 'w1', user_id: 'owner', role: 'owner' }],
      projects: [{ id: 'p1', workspace_id: 'w1', name: 'First', ref: 'project:first' }, { id: 'p2', workspace_id: 'w1', name: 'Second', ref: 'project:second' }],
      actors: [{ id: 'human-1', workspace_id: 'w1', display_name: 'Human', kind: 'human' }, { id: 'agent-1', workspace_id: 'w1', display_name: 'Agent', kind: 'agent' }],
      nodes: [
        { id: 'D-one', workspace_id: 'w1', project_id: 'p1', type: 'Decision', status: 'active', title: 'Original decision', summary: 'Choose A', body: 'Exact body\nwith newline 😀', created_by_actor_id: 'agent-1', created_at: '2026-10-10T12:00:00Z', ratified_by_actor_id: 'human-1', ratified_by_user_id: 'owner-user', ratified_at: '2026-10-11T12:00:00Z', metadata: { owner: 'unchanged' } },
        { id: 'E-one', workspace_id: 'w1', project_id: 'p1', type: 'Evidence', status: 'observation', title: 'Test data', summary: 'A passed' },
        { id: 'D-two', workspace_id: 'w1', project_id: 'p2', type: 'Decision', status: 'candidate', title: 'Another project proposal' },
      ],
      edges: [
        { id: 'edge1', workspace_id: 'w1', source_id: 'E-one', target_id: 'D-one', edge_type: 'supports', metadata: {} },
        { id: 'edge2', workspace_id: 'w1', source_id: 'D-one', target_id: 'D-two', edge_type: 'depends_on', metadata: { original: true } },
      ],
      audit_events: [{ id: 'audit1', workspace_id: 'w1', actor_id: 'human-1', actor_kind: 'human', actor_label: 'Human', object_type: 'node', object_id: 'D-one', action: 'ratified', before_state: { status: 'candidate' }, after_state: { status: 'active' }, created_at: '2026-10-11T12:00:00Z' }],
    },
  };
}

describe('shared portable core', () => {
  it('explicitly upgrades both old formats without inventing ownership or authority', () => {
    for (const version of ['0.1.0', '0.2.0']) {
      const original = { ...oldDemo(), schemaVersion: version };
      const upgraded = upgradeBundle(original);
      expect(upgraded.schemaVersion).toBe('0.3.0');
      expect(upgraded.namespace).toEqual({ datasetId: original.id, workspaceId: null, projectId: null });
      expect(upgraded.entries).toEqual(original.entries); expect(upgraded.sources).toEqual(original.sources);
      expect(upgraded.commitmentReceipts).toEqual([]); expect(upgraded.authorityGrants).toEqual([]);
      expect(original.schemaVersion).toBe(version);
    }
  });
  it('rejects unsupported formats and old-format authority smuggling', () => {
    expect(validateBundle({ ...oldDemo(), schemaVersion: '1.0.0' }).ok).toBe(false);
    expect(validateBundle({ ...oldDemo(), commitmentReceipts: [] }).ok).toBe(false);
    expect(validateBundle({ ...oldDemo(), runs: [{ ...oldDemo().runs[0], omissions: [] }] }).errors.join(' ')).toContain('omissions require');
  });
  it('validates source ranges, UTF-16 boundaries and direct attribution', () => {
    const bundle = demo(); bundle.sources[0].content = 'A😀B'; bundle.entries[0].provenance.sourceSpans = [{ messageId: 'S1', start: 1, end: 2 }];
    expect(validateBundle(bundle).errors.join(' ')).toContain('surrogate pair');
    const attribution = demo(); attribution.contexts[0].kind = 'discussion'; attribution.entries[0].provenance.expressedBy = 'demo-recorder';
    expect(validateBundle(attribution).errors.join(' ')).toContain('direct attribution does not match');
  });
  it('preserves grouped connections and inverse navigation', () => {
    const bundle = demo(), view = getView(bundle); const grouped = bundle.connections.find(c => c.id === 'C-support:v1')!;
    expect(grouped.participants).toHaveLength(4);
    for (const participant of grouped.participants) expect(relatedConnections(view, participant.entryId).map(c => c.id)).toContain(grouped.id);
    expect(relationLinks(grouped).filter(link => link.source === grouped.id).map(link => link.target)).toEqual(['E-recommendation:v1']);
  });
  it('does not leak later evidence or unanchored metadata into historical packets', () => {
    const bundle = demo(), packet = buildEvaluationPacket(bundle, 'R2', 3);
    expect(packet.entries.some(entry => entry.id === 'E-new-test:v1')).toBe(false);
    expect(packet.uncertainties).toEqual(bundle.runs[0].uncertainties);
    expect(packet.sources.some(source => source.id === 'S4')).toBe(false);
  });
  it('keeps positions stable when viewing a historical slice', () => {
    const bundle = demo(), full = buildGraphModel(bundle, getView(bundle));
    const slice = projectGraphModel(full, getView(bundle, 'R2', 3));
    for (const node of slice.nodes) expect(node.position).toEqual(full.nodes.find(item => item.id === node.id)?.position);
  });
  it('merges idempotently, rejects immutable edits, and survives JSON round trips', () => {
    const bundle = demo(); expect(mergeBundles(bundle, JSON.parse(JSON.stringify(bundle)))).toEqual(bundle);
    const edited = structuredClone(bundle); edited.entries[0].summary = 'Changed version';
    expect(() => mergeBundles(bundle, edited)).toThrow('Immutable entry conflict');
    expect(entryText(bundle, bundle.entries[0])).toBe(entryText(upgradeBundle(JSON.parse(JSON.stringify(bundle))), bundle.entries[0]));
  });
  it('preserves original contributions when extraction is revised', () => {
    const bundle = demo(), original = bundle.entries[0];
    const replacement = { ...structuredClone(original), id: `${original.logicalId}:v2`, version: 2, summary: 'Revised interpretation' };
    // Replace every affected connection explicitly as the same extraction correction.
    const connections = bundle.connections.filter(connection => connection.participants.some(p => p.entryId === original.id)).map(connection => ({ ...structuredClone(connection), id: `${connection.logicalId}:v2`, version: 2, participants: connection.participants.map(p => ({ ...p, entryId: p.entryId === original.id ? replacement.id : p.entryId })) }));
    bundle.entries.push(replacement); bundle.connections.push(...connections);
    bundle.runs.push({ ...structuredClone(bundle.runs[1]), id: 'R3', parentId: 'R2', sourceMessageIds: [], addedEntryIds: [replacement.id], addedConnectionIds: connections.map(c => c.id), replacements: [{ kind: 'entry', previousId: original.id, nextId: replacement.id }, ...connections.map(c => ({ kind: 'connection' as const, previousId: `${c.logicalId}:v1`, nextId: c.id }))] });
    expect(validateBundle(bundle).errors).toEqual([]);
    expect(getView(bundle, 'R2').entries).toContainEqual(original);
    expect(getView(bundle, 'R3').entries).toContainEqual(replacement);
    expect(bundle.entries).toContainEqual(original);
  });
  it('checks namespace identity and rejects cross-workspace reference disclosure', () => {
    const bundle = demo(); bundle.namespace.workspaceId = 'w1'; expect(validateBundle(bundle).ok).toBe(false);
    bundle.namespace.projectId = 'p1'; bundle.externalEntryRefs.push({ id: 'foreign', namespace: { workspaceId: 'w2', projectId: 'p2', datasetId: 'other' }, entryId: 'e', originalId: 'old' });
    expect(validateBundle(bundle).errors.join(' ')).toContain('cross-workspace');
  });
  it('never confers authority from portable receipts and rejects illustrative commitments', () => {
    const bundle = demo(); const receipt: CommitmentReceipt = { id: 'receipt1', action: 'adopt', actorId: 'fictional-human', entryId: 'E-commitment:v1', priorReceiptId: null, grantId: null, recordedAt: timestamp, rationale: 'Review', connectionId: null, basis: 'authorized_action', sourceSpans: [] };
    bundle.commitmentReceipts.push(receipt);
    expect(validateBundle(bundle).errors.join(' ')).toContain('illustration cannot');
    bundle.contexts[0].kind = 'discussion';
    expect(validateBundle(bundle).warnings.join(' ')).toContain('untrusted on import');
    expect(getView(bundle).entries.find(e => e.id === receipt.entryId)).toEqual(bundle.entries.find(e => e.id === receipt.entryId));
  });
  it('preserves a human commitment when an authorized agent records a new revision, including later revocation', () => {
    const bundle = demo(); bundle.contexts[0].kind = 'discussion';
    bundle.namespace = { datasetId: bundle.id, workspaceId: 'w1', projectId: 'p1' };
    bundle.authorityGrants.push({ id: 'grant1', action: 'grant', agentActorId: 'fictional-agent', ownerActorId: 'fictional-human', projectId: 'p1', priorGrantId: null, recordedAt: '2026-10-06T10:05:00-07:00' });
    const initial: CommitmentReceipt = { id: 'receipt1', action: 'adopt', actorId: 'fictional-human', entryId: 'E-commitment:v1', priorReceiptId: null, grantId: null, recordedAt: '2026-10-06T10:00:00-07:00', rationale: 'Initial choice', connectionId: null, basis: 'authorized_action', sourceSpans: [] };
    bundle.commitmentReceipts.push(initial, { ...initial, id: 'receipt2', action: 'revise', entryId: 'E-objection:v1', actorId: 'fictional-agent', priorReceiptId: initial.id, grantId: 'grant1', recordedAt: '2026-10-09T11:00:00-07:00', rationale: 'New evidence changes our commitment' });
    bundle.authorityGrants.push({ ...bundle.authorityGrants[0], id: 'revocation1', action: 'revoke', priorGrantId: 'grant1', recordedAt: '2026-10-10T11:00:00-07:00' });
    expect(validateBundle(bundle).errors).toEqual([]); expect(bundle.commitmentReceipts[0]).toEqual(initial);
    bundle.commitmentReceipts[1].recordedAt = '2026-10-11T11:00:00-07:00';
    expect(validateBundle(bundle).errors.join(' ')).toContain('valid grant at the action time');
  });
  it('rejects a fork of the same commitment receipt and cycles', () => {
    const bundle = demo(); bundle.contexts[0].kind = 'discussion';
    const first: CommitmentReceipt = { id: 'one', action: 'adopt', entryId: 'E-commitment:v1', actorId: 'fictional-human', priorReceiptId: null, grantId: null, recordedAt: timestamp, rationale: '', connectionId: null, basis: 'authorized_action', sourceSpans: [] };
    const second: CommitmentReceipt = { ...first, id: 'two', action: 'revise', entryId: 'E-objection:v1', priorReceiptId: 'one' };
    bundle.commitmentReceipts = [first, second, { ...second, id: 'three' }];
    expect(validateBundle(bundle).errors.join(' ')).toContain('Commitment successor receipts');
    bundle.commitmentReceipts = [{ ...first, action: 'revise', priorReceiptId: 'two' }, second];
    expect(validateBundle(bundle).errors.join(' ')).toContain('lineage contains a cycle');
  });
});

describe('legacy preservation converter', () => {
  it('preserves all nodes, edges, actors, projects, audits and exact payloads with deterministic mappings', () => {
    const input = snapshot(), converted = convertLegacySnapshot(input);
    expect(converted.report.ready).toBe(true); expect(converted.report.converted).toEqual({ nodes: 3, edges: 2, audits: 1 });
    for (const table of ['nodes', 'edges', 'actors', 'projects', 'audit_events'] as const) for (const original of input.tables[table]) {
      const mappings = converted.report.mappings.filter(m => m.table === table && m.legacyId === original.id);
      expect(mappings.length).toBeGreaterThan(0);
      for (const mapping of mappings) expect(converted.bundles.find(b => b.id === mapping.datasetId)!.sources.find(s => s.id === mapping.sourceId)!.provenance!.payload).toEqual(original);
    }
    expect(convertLegacySnapshot(input)).toEqual(converted);
    for (const bundle of converted.bundles) expect(mergeBundles(bundle, bundle)).toEqual(bundle);
    const decision = converted.bundles[0].entries.find(entry => entry.id.includes('D-one'))!;
    expect(entryText(converted.bundles[0], decision)).toContain('"owner":"unchanged"');
    expect(entryText(converted.bundles[0], decision)).toContain('Exact body\nwith newline 😀');
  });
  it('never turns database creators or creation timestamps into authorship or occurrence times', () => {
    const bundle = convertLegacySnapshot(snapshot()).bundles[0];
    for (const entry of bundle.entries) { expect(entry.provenance.expressedBy).toBeNull(); expect(entry.provenance.occurredAt).toBeNull(); expect(entry.provenance.recordedBy).toBe('import:legacy-converter'); }
    expect(bundle.sources.every(source => source.role === 'record' || source.role === 'audit')).toBe(true);
    expect(bundle.sources.find(source => source.provenance?.originalRecord?.id === 'D-one')!.loggedAt).toBe('2026-10-10T12:00:00Z');
  });
  it('preserves human approvals separately from import attribution and inactive status', () => {
    const input = snapshot(); input.tables.nodes[0].status = 'superseded'; const result = convertLegacySnapshot(input), receipt = result.bundles[0].commitmentReceipts[0];
    expect(receipt.actorId).toBe('human-1'); expect(receipt.recordedAt).toBe('2026-10-11T12:00:00Z');
    expect(result.report.legacyApprovals[0].eligibleForCurrent).toBe(false);
    expect(receipt.basis).toBe('legacy_approval');
    const incomplete = snapshot(); delete incomplete.tables.nodes[0].ratified_at;
    expect(convertLegacySnapshot(incomplete).report.ready).toBe(true);
    expect(convertLegacySnapshot(incomplete).bundles[0].commitmentReceipts[0]).toMatchObject({ basis: 'legacy_state', actorId: 'unknown:legacy-origin', recordedAt: timestamp });
    const missingUser = snapshot(); delete missingUser.tables.nodes[0].ratified_by_user_id;
    expect(convertLegacySnapshot(missingUser).report.ready).toBe(true);
    expect(convertLegacySnapshot(missingUser).report.legacyApprovals[0].eligibleForCurrent).toBe(true);
    expect(convertLegacySnapshot(missingUser).report.concerns[0].kind).toBe('legacy_authority_identity_unresolved');
  });
  it('retains cross-project edges once with external pointers and exact legacy relation', () => {
    const result = convertLegacySnapshot(snapshot()), bundle = result.bundles[0], cross = bundle.connections.find(c => c.relation === 'legacy:depends_on')!;
    expect(cross.participants.map(p => p.role)).toEqual(['source', 'target']);
    expect(bundle.externalEntryRefs).toHaveLength(1); expect(bundle.entries).toHaveLength(2);
    expect(getView(bundle).externalConnections).toEqual([cross]);
    expect(relatedConnections(getView(bundle), cross.participants[0].entryId)).toContainEqual(cross);
    expect(buildEvaluationPacket(bundle).connections.map(c => c.id)).toContain(cross.id);
    expect(result.bundles.flatMap(b => b.connections).filter(c => c.id === cross.id)).toHaveLength(1);
  });
  it('does not expose raw legacy metadata or complete audit payloads in a derived evaluation packet', () => {
    const bundle = convertLegacySnapshot(snapshot()).bundles[0], packet = buildEvaluationPacket(bundle);
    expect(packet.sources.some(source => 'payload' in (source.provenance ?? {}))).toBe(false);
    expect(packet.sources.some(source => 'content' in source)).toBe(false);
  });
  it('blocks unmapped ownership, missing endpoints and duplicate source identities', () => {
    const missing = snapshot(); delete missing.tables.nodes[0].project_id;
    const result = convertLegacySnapshot(missing); expect(result.report.ready).toBe(false); expect(result.report.unassignedNodes[0].id).toBe('D-one');
    const duplicate = snapshot(); duplicate.tables.nodes.push(structuredClone(duplicate.tables.nodes[0]));
    expect(() => convertLegacySnapshot(duplicate)).toThrow('Duplicate snapshot identity');
    const endpoint = snapshot(); endpoint.tables.edges[0].target_id = 'missing'; expect(convertLegacySnapshot(endpoint).report.ready).toBe(false);
  });
  it('retains workspace and deleted-object audits instead of guessing ownership', () => {
    const input = snapshot(); input.tables.audit_events.push({ id: 'deleted', workspace_id: 'w1', object_id: 'gone', object_type: 'node', action: 'delete' });
    const result = convertLegacySnapshot(input); expect(result.report.retainedUnassignedAudits).toEqual([input.tables.audit_events[1]]);
    expect(result.report.concerns.some(item => item.kind === 'unassigned_audit')).toBe(true);
  });
  it('accepts explicit missing-ownership resolution without rewriting the source row', () => {
    const input = snapshot(); delete input.tables.nodes[0].project_id;
    const assignment = { workspaceId: 'w1', nodeId: 'D-one', projectId: 'p1', reason: 'Owner supplied project assignment' };
    const result = convertLegacySnapshot(input, { projectAssignments: [assignment] });
    expect(result.report.ready).toBe(true); expect(result.report.explicitProjectAssignments).toEqual([assignment]);
    expect(result.bundles[0].sources.find(source => source.provenance?.originalRecord?.table === 'nodes' && source.provenance.originalRecord.id === 'D-one')!.provenance!.payload).toEqual(input.tables.nodes[0]);
    expect(() => convertLegacySnapshot(snapshot(), { projectAssignments: [assignment] })).toThrow('cannot overwrite existing ownership');
  });
});
