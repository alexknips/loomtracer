import { z } from 'zod';
import type { Connection, Entry, ExtractionRun, LegacyReasoningBundle, NativeRelation, ReasoningBundle, Relation, ValidationResult, View } from './types';

const id = z.string().min(1);
const instant = z.string().refine(value => /(?:Z|[+-]\d{2}:\d{2})$/.test(value) && Number.isFinite(Date.parse(value)), 'Expected an ISO timestamp with time zone');
const namespaceSchema = z.object({ datasetId: id, workspaceId: id.nullable(), projectId: id.nullable() }).strict();
const spanSchema = z.object({ messageId: id, start: z.number().int().nonnegative(), end: z.number().int().nonnegative() }).strict();
const provenanceSchema = z.object({ expressedBy: id.nullable(), recordedBy: id, occurredAt: instant.nullable(), recordedAt: instant, basis: z.enum(['direct', 'inferred']), sourceSpans: z.array(spanSchema) }).strict();
const versioned = { id, logicalId: id, version: z.number().int().positive(), contextId: id, provenance: provenanceSchema };
const entrySchema = z.object({ ...versioned, content: z.string().optional(), summary: z.string().optional(), labels: z.array(id) }).strict();
const connectionSchema = z.object({ ...versioned, relation: z.union([z.enum(['responds_to', 'answers', 'supports', 'challenges', 'compares', 'adopts', 'revises', 'withdraws']), z.string().regex(/^legacy:[a-zA-Z0-9_.-]+$/)]), participants: z.array(z.object({ entryId: id, role: id }).strict()), rationaleEntryId: id.optional() }).strict();
export const bundleSchema = z.object({
  schemaVersion: z.literal('0.3.0'), id, title: id, namespace: namespaceSchema,
  actors: z.array(z.object({ id, name: id, kind: z.enum(['human', 'agent', 'system', 'import', 'unknown']) }).strict()),
  contexts: z.array(z.object({ id, title: id, kind: z.enum(['discussion', 'illustration', 'quoted_source', 'legacy_record']) }).strict()),
  sources: z.array(z.object({ id, originalId: id, actorId: id, role: z.enum(['user', 'assistant', 'record', 'audit']), content: z.string(), occurredAt: instant.nullable(), loggedAt: instant.nullable(), order: z.number().int().nonnegative(), sourceRef: id, attachmentIds: z.array(id), active: z.boolean(), supersededBy: id.optional(), provenance: z.object({ kind: z.enum(['chat', 'legacy_record', 'legacy_audit', 'authored']), originalRecord: z.object({ table: id, id, workspaceId: id }).strict().optional(), recordedBy: id.nullable().optional(), recordedAt: instant.nullable().optional(), payload: z.json().optional() }).strict().optional() }).strict()),
  attachments: z.array(z.object({ id, mimeType: z.enum(['image/png', 'image/jpeg', 'image/webp']), dataUrl: z.string().regex(/^data:image\/(png|jpeg|webp);base64,[A-Za-z0-9+/=]+$/), sha256: z.string().regex(/^[a-f0-9]{64}$/i) }).strict()),
  manifest: z.object({ threadId: id, frozenAt: instant, cutoffDescription: id, segments: z.array(z.object({ ref: id, sha256: z.string().regex(/^[a-f0-9]{64}$/i), selectedMessageIds: z.array(id) }).strict()), exclusions: z.array(z.string()) }).strict(),
  entries: z.array(entrySchema), connections: z.array(connectionSchema),
  runs: z.array(z.object({ id, parentId: id.nullable(), createdAt: instant, recordedBy: id, sourceCutoff: z.number().int().nonnegative(), sourceMessageIds: z.array(id), addedEntryIds: z.array(id), addedConnectionIds: z.array(id), replacements: z.array(z.object({ kind: z.enum(['entry', 'connection']), previousId: id, nextId: id }).strict()), omissions: z.array(z.object({ kind: z.enum(['entry', 'connection']), recordId: id, reason: z.string().refine(value => value.trim().length > 0, 'Omission needs a nonblank extraction-correction reason') }).strict()).optional(), uncertainties: z.array(z.string()), notes: z.string(), reviewResolutions: z.array(z.object({ noteId: id, status: z.enum(['addressed', 'unresolved']), comment: id }).strict()) }).strict()),
  reviewNotes: z.array(z.object({ id, reviewer: id, createdAt: instant, runId: id, target: z.object({ kind: z.enum(['entry', 'connection', 'source', 'missing']), id: id.nullable() }).strict(), sourceSpans: z.array(spanSchema), comment: id }).strict()),
  authorityGrants: z.array(z.object({ id, action: z.enum(['grant', 'revoke']), agentActorId: id, ownerActorId: id, projectId: id, recordedAt: instant, priorGrantId: id.nullable() }).strict()),
  commitmentReceipts: z.array(z.object({ id, action: z.enum(['adopt', 'revise', 'withdraw']), entryId: id, priorReceiptId: id.nullable(), actorId: id, grantId: id.nullable(), recordedAt: instant, rationale: z.string(), connectionId: id.nullable(), basis: z.enum(['authorized_action', 'legacy_approval', 'legacy_state']), sourceSpans: z.array(spanSchema) }).strict()),
  externalEntryRefs: z.array(z.object({ id, namespace: namespaceSchema, entryId: id, originalId: id }).strict()),
}).strict();

/** Small append requests carry only new records. The server derives lineage and coverage. */
export const deltaSchema = z.object({
  actors: bundleSchema.shape.actors.optional(), contexts: bundleSchema.shape.contexts.optional(),
  sources: bundleSchema.shape.sources.optional(), attachments: bundleSchema.shape.attachments.optional(),
  entries: bundleSchema.shape.entries.optional(), connections: bundleSchema.shape.connections.optional(),
  reviewNotes: bundleSchema.shape.reviewNotes.optional(), externalEntryRefs: bundleSchema.shape.externalEntryRefs.optional(),
  run: bundleSchema.shape.runs.element.pick({ id: true, createdAt: true, recordedBy: true, notes: true,
    uncertainties: true, replacements: true, omissions: true, reviewResolutions: true })
    .partial({ uncertainties: true, replacements: true, omissions: true, reviewResolutions: true }),
}).strict();
export type ReasoningDelta = z.infer<typeof deltaSchema>;

type RoleRule = { min: number; max?: number };
export const relationContracts: Record<NativeRelation, { roles: Record<string, RoleRule> }> = {
  responds_to: { roles: { response: { min: 1, max: 1 }, target: { min: 1 } } },
  answers: { roles: { answer: { min: 1, max: 1 }, question: { min: 1 } } },
  supports: { roles: { premise: { min: 0 }, assumption: { min: 0 }, evidence: { min: 0 }, criterion: { min: 0 }, conclusion: { min: 1, max: 1 } } },
  challenges: { roles: { objection: { min: 1, max: 1 }, target: { min: 1 } } },
  compares: { roles: { assessment: { min: 1, max: 1 }, alternative: { min: 2 }, criterion: { min: 0 } } },
  adopts: { roles: { commitment: { min: 1, max: 1 }, proposal: { min: 1 } } },
  revises: { roles: { revision: { min: 1, max: 1 }, previous: { min: 1 } } },
  withdraws: { roles: { withdrawal: { min: 1, max: 1 }, target: { min: 1 } } },
};
export function relationContract(relation: Relation): { roles: Record<string, RoleRule> } {
  return relation.startsWith('legacy:') ? { roles: { source: { min: 1, max: 1 }, target: { min: 1, max: 1 } } } : relationContracts[relation as NativeRelation];
}

/** Explicit, lossless version conversion. Old bundles cannot smuggle hosted authority through upgrade. */
function upgradeInput(input: unknown): unknown {
  if (!input || typeof input !== 'object') return input;
  const value = input as Record<string, unknown>;
  if (value.schemaVersion !== '0.1.0' && value.schemaVersion !== '0.2.0') return input;
  if (['namespace', 'authorityGrants', 'commitmentReceipts', 'externalEntryRefs'].some(key => key in value)) throw new Error('Older formats cannot carry 0.3 hosted ownership or authority fields');
  if (value.schemaVersion === '0.1.0' && Array.isArray(value.runs) && value.runs.some(run => run && typeof run === 'object' && 'omissions' in run)) throw new Error('omissions require schemaVersion 0.2.0; 0.1.0 does not define extraction omissions');
  return { ...value, schemaVersion: '0.3.0', namespace: { datasetId: value.id, workspaceId: null, projectId: null }, authorityGrants: [], commitmentReceipts: [], externalEntryRefs: [] };
}

export function upgradeBundle(input: unknown): ReasoningBundle {
  const result = validateBundle(input);
  if (!result.ok) throw new Error(`Invalid reasoning bundle: ${result.errors.join('; ')}`);
  return result.bundle!;
}

function ancestry(bundle: ReasoningBundle, runId?: string): ExtractionRun[] {
  if (!bundle.runs.length) throw new Error('This bundle has source messages but no extraction run. Add an extraction before inspecting its reasoning.');
  const map = new Map(bundle.runs.map(run => [run.id, run]));
  let current = map.get(runId ?? bundle.runs[bundle.runs.length - 1].id);
  if (!current) throw new Error(`Unknown extraction run: ${runId}`);
  const result: ExtractionRun[] = [];
  const seen = new Set<string>();
  while (current) {
    if (seen.has(current.id)) throw new Error(`Extraction lineage contains a cycle at ${current.id}`);
    seen.add(current.id); result.unshift(current);
    if (current.parentId === null) break;
    const parent = map.get(current.parentId);
    if (!parent) throw new Error(`Unknown parent run: ${current.parentId}`);
    current = parent;
  }
  return result;
}

function accumulated(bundle: ReasoningBundle, runId?: string) {
  const lineage = ancestry(bundle, runId);
  const entryIds = new Set<string>(); const connectionIds = new Set<string>();
  for (const run of lineage) {
    run.addedEntryIds.forEach(item => entryIds.add(item));
    run.addedConnectionIds.forEach(item => connectionIds.add(item));
    for (const replacement of run.replacements) (replacement.kind === 'entry' ? entryIds : connectionIds).delete(replacement.previousId);
    for (const omission of run.omissions ?? []) (omission.kind === 'entry' ? entryIds : connectionIds).delete(omission.recordId);
  }
  return { entryIds, connectionIds, lineage };
}

/** Resolve canonical text; generated summaries never replace the cited content. */
export function entryText(bundle: ReasoningBundle, entry: Entry): string {
  if (entry.content !== undefined) return entry.content;
  const sources = new Map(bundle.sources.map(source => [source.id, source]));
  return entry.provenance.sourceSpans.map(span => {
    const source = sources.get(span.messageId);
    if (!source) throw new Error(`Missing source ${span.messageId}`);
    return source.content.slice(span.start, span.end);
  }).join('\n[…]\n');
}

/** Source cutoff is conversation order, never the extraction/import timestamp. */
export function getView(bundle: ReasoningBundle, runId?: string, cutoff?: number): View {
  const { entryIds, connectionIds, lineage } = accumulated(bundle, runId);
  const run = lineage[lineage.length - 1];
  const limit = Math.min(cutoff ?? run.sourceCutoff, run.sourceCutoff);
  const covered = new Set(lineage.flatMap(item => item.sourceMessageIds));
  const sourceMap = new Map(bundle.sources.map(source => [source.id, source]));
  const visibleProvenance = (record: Entry | Connection) => {
    if (record.provenance.sourceSpans.length) return record.provenance.sourceSpans.every(span => {
      const source = sourceMap.get(span.messageId);
      return source?.active && source.order <= limit && covered.has(source.id);
    });
    // An authored contribution has no source position. Reveal it only at the cutoff
    // of the run that introduced it, rather than back-projecting it into history.
    const introduction = lineage.find(item => item.addedEntryIds.includes(record.id) || item.addedConnectionIds.includes(record.id));
    return !!introduction && introduction.sourceCutoff <= limit;
  };
  const entries = bundle.entries.filter(entry => entryIds.has(entry.id) && visibleProvenance(entry));
  const visibleIds = new Set(entries.map(entry => entry.id));
  const connections = bundle.connections.filter(connection => connectionIds.has(connection.id) && visibleProvenance(connection) && connection.participants.every(participant => visibleIds.has(participant.entryId)) && (!connection.rationaleEntryId || visibleIds.has(connection.rationaleEntryId)));
  const externalIds = new Set(bundle.externalEntryRefs.map(ref => ref.id));
  const externalConnections = bundle.connections.filter(connection => connectionIds.has(connection.id) && visibleProvenance(connection) && connection.participants.some(participant => externalIds.has(participant.entryId)) && connection.participants.every(participant => visibleIds.has(participant.entryId) || externalIds.has(participant.entryId)) && (!connection.rationaleEntryId || visibleIds.has(connection.rationaleEntryId)));
  return { entries, connections, externalConnections, run, cutoff: limit };
}

/** The same stored assertion is discoverable from every participant. */
export function relatedConnections(view: View, entryId: string): Connection[] {
  return [...view.connections, ...(view.externalConnections ?? [])].filter(connection => connection.participants.some(participant => participant.entryId === entryId) || connection.rationaleEntryId === entryId);
}

/** A deliberately bounded evaluator input: no full transcript, notes, answer key, or attachment bytes. */
export function buildEvaluationPacket(bundle: ReasoningBundle, runId?: string, cutoff?: number) {
  const view = getView(bundle, runId, cutoff);
  const allConnections = [...view.connections, ...(view.externalConnections ?? [])];
  const visibleIds = new Set(view.entries.map(entry => entry.id));
  const coveredSources = new Set(ancestry(bundle, view.run.id).flatMap(run => run.sourceMessageIds));
  const sourceMap = new Map(bundle.sources.map(source => [source.id, source]));
  const commitmentReceipts = bundle.commitmentReceipts.filter(receipt => visibleIds.has(receipt.entryId) && (receipt.sourceSpans.length ? receipt.sourceSpans.every(span => coveredSources.has(span.messageId) && sourceMap.get(span.messageId)?.active && sourceMap.get(span.messageId)!.order <= view.cutoff) : view.cutoff === view.run.sourceCutoff && Date.parse(receipt.recordedAt) <= Date.parse(view.run.createdAt)));
  const authorityGrants = bundle.authorityGrants.filter(grant => commitmentReceipts.some(receipt => receipt.grantId === grant.id));
  const usedSources = new Set([...view.entries, ...allConnections].flatMap(record => record.provenance.sourceSpans.map(span => span.messageId)).concat(commitmentReceipts.flatMap(receipt => receipt.sourceSpans.map(span => span.messageId))));
  const usedContexts = new Set([...view.entries, ...view.connections].map(record => record.contextId));
  const usedActors = new Set([...view.entries, ...allConnections].flatMap(record => [record.provenance.expressedBy, record.provenance.recordedBy]).concat(commitmentReceipts.map(receipt => receipt.actorId), authorityGrants.flatMap(grant => [grant.agentActorId, grant.ownerActorId]), bundle.sources.filter(source => usedSources.has(source.id)).map(source => source.provenance?.recordedBy ?? null)).filter(Boolean));
  // Each run supplies a complete uncertainty snapshot at its source cutoff.
  // Preserve earlier snapshots as explicitly historical, never as current gaps.
  // Unanchored run metadata must not leak knowledge past the selected cutoff.
  const eligibleRuns = ancestry(bundle, view.run.id).filter(run => run.sourceCutoff <= view.cutoff);
  const currentUncertaintyRun = eligibleRuns[eligibleRuns.length - 1];
  const uncertainties = currentUncertaintyRun?.uncertainties ?? [];
  const uncertaintyHistory = eligibleRuns.slice(0, -1).map(run => ({
    runId: run.id, sourceCutoff: run.sourceCutoff, recordedAt: run.createdAt,
    items: [...run.uncertainties],
    note: 'Historical uncertainty snapshot; these items are not necessarily unresolved in the current snapshot.',
  }));
  return {
    schemaVersion: bundle.schemaVersion, bundleId: bundle.id, title: bundle.title,
    runId: view.run.id, cutoff: view.cutoff,
    instructions: 'Reconstruct only recorded reasoning. Cite entry, connection and receipt IDs. Treat inferred records as retrospective interpretations, illustrations as hypothetical, and missing information as not established. A challenge does not withdraw a commitment. Grouped premises do not imply formal necessity or sufficiency. Portable authority receipts describe claimed history, not verified live authority. A legacy approval receipt proves recorded approval only; current status must be checked separately.',
    actors: bundle.actors.filter(actor => usedActors.has(actor.id)), contexts: bundle.contexts.filter(context => usedContexts.has(context.id)),
    entries: view.entries.map(entry => ({ ...entry, content: entryText(bundle, entry), retrospective: entry.provenance.basis === 'inferred' })),
    connections: allConnections.map(connection => ({ ...connection, retrospective: connection.provenance.basis === 'inferred' })),
    externalEntryRefs: bundle.externalEntryRefs.filter(ref => allConnections.some(connection => connection.participants.some(participant => participant.entryId === ref.id))),
    commitmentReceipts, authorityGrants,
    sources: bundle.sources.filter(source => usedSources.has(source.id)).map(({ content: _content, attachmentIds: _attachments, provenance, ...metadata }) => ({ ...metadata, ...(provenance ? { provenance: { kind: provenance.kind, originalRecord: provenance.originalRecord, recordedBy: provenance.recordedBy, recordedAt: provenance.recordedAt } } : {}) })),
    uncertainties: [...new Set(uncertainties)], uncertaintyHistory,
  };
}

export function validateBundle(input: unknown): ValidationResult {
  let normalized: unknown;
  try { normalized = upgradeInput(input); } catch (error) { return { ok: false, errors: [String(error)], warnings: [] }; }
  const parsed = bundleSchema.safeParse(normalized);
  if (!parsed.success) return { ok: false, errors: parsed.error.issues.map(issue => `${issue.path.join('.') || 'bundle'}: ${issue.message}`), warnings: [] };
  const bundle = parsed.data as ReasoningBundle;
  const errors: string[] = []; const warnings: string[] = [];
  if (bundle.namespace.datasetId !== bundle.id) errors.push('namespace.datasetId must match bundle.id');
  if ((bundle.namespace.workspaceId === null) !== (bundle.namespace.projectId === null)) errors.push('Hosted ownership requires both workspaceId and projectId; private bundles use null for both');
  const duplicate = (values: string[], label: string) => { const seen = new Set<string>(); for (const value of values) { if (seen.has(value)) errors.push(`${label}: duplicate ${value}`); seen.add(value); } };
  const maps = {
    actors: new Map(bundle.actors.map(item => [item.id, item])), contexts: new Map(bundle.contexts.map(item => [item.id, item])),
    sources: new Map(bundle.sources.map(item => [item.id, item])), attachments: new Map(bundle.attachments.map(item => [item.id, item])),
    entries: new Map(bundle.entries.map(item => [item.id, item])), connections: new Map(bundle.connections.map(item => [item.id, item])),
    runs: new Map(bundle.runs.map(item => [item.id, item])), notes: new Map(bundle.reviewNotes.map(item => [item.id, item])),
    grants: new Map(bundle.authorityGrants.map(item => [item.id, item])), receipts: new Map(bundle.commitmentReceipts.map(item => [item.id, item])), external: new Map(bundle.externalEntryRefs.map(item => [item.id, item])),
  };
  const lists = [bundle.actors, bundle.contexts, bundle.sources, bundle.attachments, bundle.entries, bundle.connections, bundle.runs, bundle.reviewNotes, bundle.authorityGrants, bundle.commitmentReceipts, bundle.externalEntryRefs];
  duplicate(lists.flatMap(list => list.map(item => item.id)), 'Record IDs');
  // Original message identity belongs to the conversation, not the log copy
  // that happens to contain it. Replayed files must not create new occurrences.
  duplicate(bundle.sources.map(item => item.originalId), 'Source identities');
  duplicate(bundle.sources.filter(item => item.active).map(item => String(item.order)), 'Active source order');
  duplicate(bundle.manifest.segments.map(segment => segment.ref), 'Manifest segments');
  const exists = (set: Map<string, unknown>, value: string | null | undefined, label: string) => { if (value !== null && value !== undefined && !set.has(value)) errors.push(`${label}: unknown reference ${value}`); };
  const checkSpans = (spans: Entry['provenance']['sourceSpans'], label: string) => {
    for (const span of spans) {
      const source = maps.sources.get(span.messageId);
      if (!source) { errors.push(`${label}: unknown source ${span.messageId}`); continue; }
      if (span.start >= span.end || span.end > source.content.length) errors.push(`${label}: invalid UTF-16 span ${span.messageId}[${span.start},${span.end})`);
      // A range must not split a surrogate pair: it would not preserve the source character.
      for (const offset of [span.start, span.end]) {
        const before = source.content.charCodeAt(offset - 1), at = source.content.charCodeAt(offset);
        if (before >= 0xd800 && before <= 0xdbff && at >= 0xdc00 && at <= 0xdfff) errors.push(`${label}: UTF-16 span splits a surrogate pair`);
      }
    }
  };
  const checkProvenance = (record: Entry | Connection) => {
    exists(maps.contexts, record.contextId, record.id); exists(maps.actors, record.provenance.expressedBy, record.id); exists(maps.actors, record.provenance.recordedBy, record.id);
    checkSpans(record.provenance.sourceSpans, record.id);
    if (record.provenance.basis === 'inferred') warnings.push(`${record.id}: retrospective interpretation; do not attribute it to the original decision maker.`);
    if (record.provenance.expressedBy === null) warnings.push(`${record.id}: expressing actor is unknown.`);
    if (record.provenance.occurredAt === null) warnings.push(`${record.id}: occurrence time is unknown; preserve source order.`);
  };
  for (const source of bundle.sources) {
    exists(maps.actors, source.actorId, source.id); source.attachmentIds.forEach(item => exists(maps.attachments, item, source.id));
    exists(maps.actors, source.provenance?.recordedBy, source.id);
    if (source.provenance?.kind.startsWith('legacy_') && !source.provenance.originalRecord) errors.push(`${source.id}: legacy sources require an original record identity`);
    if (source.provenance?.kind === 'legacy_record' && source.role !== 'record') errors.push(`${source.id}: a legacy record is not a chat message`);
    if (source.provenance?.kind === 'legacy_audit' && source.role !== 'audit') errors.push(`${source.id}: a legacy audit is not a chat message`);
    exists(maps.sources, source.supersededBy, source.id);
    if (source.supersededBy === source.id) errors.push(`${source.id}: source cannot supersede itself`);
    if (source.active && source.supersededBy) errors.push(`${source.id}: a superseded source must be inactive`);
  }
  for (const attachment of bundle.attachments) if (!attachment.dataUrl.startsWith(`data:${attachment.mimeType};base64,`)) errors.push(`${attachment.id}: data URL media type does not match mimeType`);
  for (const segment of bundle.manifest.segments) { duplicate(segment.selectedMessageIds, segment.ref); segment.selectedMessageIds.forEach(item => exists(maps.sources, item, segment.ref)); }
  const manifested = new Set(bundle.manifest.segments.flatMap(segment => segment.selectedMessageIds));
  for (const source of bundle.sources) if (!manifested.has(source.id)) errors.push(`${source.id}: absent from frozen source manifest`);
  duplicate(bundle.entries.map(item => `${item.logicalId}@${item.version}`), 'Entry versions');
  duplicate(bundle.connections.map(item => `${item.logicalId}@${item.version}`), 'Connection versions');
  for (const entry of bundle.entries) {
    checkProvenance(entry);
    if (entry.content !== undefined && entry.provenance.sourceSpans.length) errors.push(`${entry.id}: use source spans or newly authored content, not both`);
    if (entry.content === undefined && !entry.provenance.sourceSpans.length) errors.push(`${entry.id}: entry requires source spans or authored content`);
    if (entry.content !== undefined && !entry.content.trim()) errors.push(`${entry.id}: authored content cannot be empty`);
    if (entry.provenance.basis === 'direct' && entry.provenance.occurredAt && entry.provenance.sourceSpans.length) {
      const knownSourceTimes = entry.provenance.sourceSpans.map(span => maps.sources.get(span.messageId)?.occurredAt).filter((time): time is string => !!time);
      if (knownSourceTimes.length && !knownSourceTimes.some(time => Date.parse(time) === Date.parse(entry.provenance.occurredAt!))) warnings.push(`${entry.id}: occurrence time differs from every known anchor message time; verify whether it refers to an event reported in the source.`);
    }
    if (entry.provenance.basis === 'direct' && entry.provenance.expressedBy && maps.contexts.get(entry.contextId)?.kind === 'discussion') {
      for (const span of entry.provenance.sourceSpans) if (maps.sources.has(span.messageId) && maps.sources.get(span.messageId)?.actorId !== entry.provenance.expressedBy) errors.push(`${entry.id}: direct attribution does not match source speaker; use a quoted context or an explicitly inferred record`);
    }
  }
  for (const connection of bundle.connections) {
    checkProvenance(connection); exists(maps.entries, connection.rationaleEntryId, connection.id);
    if (connection.provenance.basis === 'direct' && connection.provenance.expressedBy && connection.provenance.sourceSpans.length && maps.contexts.get(connection.contextId)?.kind === 'discussion') {
      // The source anchors may include several speakers' supporting material,
      // but one must belong to the actor who expressed this relationship.
      if (!connection.provenance.sourceSpans.some(span => maps.sources.get(span.messageId)?.actorId === connection.provenance.expressedBy)) errors.push(`${connection.id}: direct relationship attribution has no matching source speaker`);
    }
    const contract = relationContract(connection.relation).roles;
    duplicate(connection.participants.map(item => `${item.entryId}\u0000${item.role}`), connection.id);
    if (new Set(connection.participants.map(item => item.entryId)).size < 2) errors.push(`${connection.id}: a connection needs at least two distinct entry IDs`);
    for (const participant of connection.participants) {
      if (!connection.relation.startsWith('legacy:') || !maps.external.has(participant.entryId)) exists(maps.entries, participant.entryId, connection.id);
      if (!contract[participant.role]) errors.push(`${connection.id}: invalid role ${participant.role} for ${connection.relation}`);
    }
    if (connection.participants.some(item => maps.external.has(item.entryId))) warnings.push(`${connection.id}: cross-project legacy relationship retained; inspect the authorized external project to resolve its participant.`);
    for (const [role, bounds] of Object.entries(contract)) {
      const count = connection.participants.filter(item => item.role === role).length;
      if (count < bounds.min || (bounds.max !== undefined && count > bounds.max)) errors.push(`${connection.id}: ${connection.relation} requires ${bounds.min}${bounds.max === bounds.min ? '' : '+'} ${role} participant(s)`);
    }
    if (connection.relation === 'supports' && !connection.participants.some(item => ['premise', 'assumption', 'evidence', 'criterion'].includes(item.role))) errors.push(`${connection.id}: supports requires at least one ground`);
    if (!connection.provenance.sourceSpans.length) warnings.push(`${connection.id}: relationship has no source anchor; review its basis before treating it as expressed reasoning.`);
    if (connection.relation === 'adopts') {
      const kinds = connection.participants.map(item => maps.contexts.get(maps.entries.get(item.entryId)?.contextId ?? '')?.kind).filter(Boolean);
      if (new Set([maps.contexts.get(connection.contextId)?.kind, ...kinds]).size > 1) errors.push(`${connection.id}: adoption cannot cross discussion, illustration, or quoted-source contexts`);
      const commitment = maps.entries.get(connection.participants.find(item => item.role === 'commitment')?.entryId ?? '');
      if (connection.provenance.basis === 'direct' && commitment && connection.provenance.expressedBy !== commitment.provenance.expressedBy) errors.push(`${connection.id}: direct adoption must be attributed to the commitment's expressing actor`);
      if (connection.provenance.basis === 'inferred') warnings.push(`${connection.id}: inferred adoption is not established approval.`);
    }
  }
  const maxOrder = Math.max(0, ...bundle.sources.map(item => item.order));
  for (const run of bundle.runs) {
    exists(maps.actors, run.recordedBy, run.id); exists(maps.runs, run.parentId, run.id);
    if (run.sourceCutoff > maxOrder) errors.push(`${run.id}: source cutoff exceeds imported conversation`);
    for (const key of ['sourceMessageIds', 'addedEntryIds', 'addedConnectionIds'] as const) duplicate(run[key], `${run.id}.${key}`);
    run.sourceMessageIds.forEach(item => { exists(maps.sources, item, run.id); if ((maps.sources.get(item)?.order ?? 0) > run.sourceCutoff) errors.push(`${run.id}: covered source ${item} is after its cutoff`); });
    run.addedEntryIds.forEach(item => exists(maps.entries, item, run.id)); run.addedConnectionIds.forEach(item => exists(maps.connections, item, run.id));
    let parents: ExtractionRun[];
    try { parents = ancestry(bundle, run.id).slice(0, -1); } catch (error) { errors.push(String(error)); continue; }
    const parentState = parents.length ? accumulated(bundle, parents[parents.length - 1].id) : { entryIds: new Set<string>(), connectionIds: new Set<string>() };
    const parent = parents[parents.length - 1];
    if (parent && (run.sourceCutoff < parent.sourceCutoff || Date.parse(run.createdAt) < Date.parse(parent.createdAt))) errors.push(`${run.id}: revision cannot move its parent cutoff or creation time backwards`);
    const previousAdds = new Set(parents.flatMap(item => [...item.addedEntryIds, ...item.addedConnectionIds]));
    for (const item of [...run.addedEntryIds, ...run.addedConnectionIds]) if (previousAdds.has(item)) errors.push(`${run.id}: ${item} was already introduced in an ancestor`);
    duplicate(run.replacements.map(item => `${item.kind}:${item.previousId}`), `${run.id} replacements`);
    for (const replacement of run.replacements) {
      const records = replacement.kind === 'entry' ? maps.entries : maps.connections;
      const live = replacement.kind === 'entry' ? parentState.entryIds : parentState.connectionIds;
      const adds = replacement.kind === 'entry' ? run.addedEntryIds : run.addedConnectionIds;
      const before = records.get(replacement.previousId), after = records.get(replacement.nextId);
      if (!before || !after || !live.has(replacement.previousId) || !adds.includes(replacement.nextId)) errors.push(`${run.id}: replacement must replace an active ancestor record with a newly added record`);
      else if (before.logicalId !== after.logicalId || after.version !== before.version + 1) errors.push(`${run.id}: replacement must preserve logical ID and increment version by one`);
    }
    duplicate((run.omissions ?? []).map(item => `${item.kind}:${item.recordId}`), `${run.id} omissions`);
    for (const omission of run.omissions ?? []) {
      const records = omission.kind === 'entry' ? maps.entries : maps.connections;
      const live = omission.kind === 'entry' ? parentState.entryIds : parentState.connectionIds;
      const adds = omission.kind === 'entry' ? run.addedEntryIds : run.addedConnectionIds;
      if (!records.has(omission.recordId)) errors.push(`${run.id}: omission targets unknown ${omission.kind} ${omission.recordId}`);
      else if (!live.has(omission.recordId)) errors.push(`${run.id}: omission ${omission.recordId} must target an active parent ${omission.kind}; new, already omitted, and superseded versions cannot be omitted`);
      if (adds.includes(omission.recordId)) errors.push(`${run.id}: cannot omit newly added ${omission.kind} ${omission.recordId} in the same run`);
      if (run.replacements.some(item => item.kind === omission.kind && (item.previousId === omission.recordId || item.nextId === omission.recordId))) errors.push(`${run.id}: ${omission.recordId} cannot be both omitted and replaced in the same run`);
    }
    const covered = new Set([...parents, run].flatMap(item => item.sourceMessageIds));
    const allNew = [...run.addedEntryIds.map(item => maps.entries.get(item)), ...run.addedConnectionIds.map(item => maps.connections.get(item))].filter((item): item is Entry | Connection => !!item);
    for (const record of allNew) for (const span of record.provenance.sourceSpans) if (!covered.has(span.messageId) || (maps.sources.get(span.messageId)?.order ?? 0) > run.sourceCutoff) errors.push(`${run.id}: ${record.id} uses source outside extraction coverage`);
    const state = accumulated(bundle, run.id);
    const logicalEntries = [...state.entryIds].map(item => maps.entries.get(item)?.logicalId).filter((item): item is string => !!item);
    const logicalConnections = [...state.connectionIds].map(item => maps.connections.get(item)?.logicalId).filter((item): item is string => !!item);
    duplicate(logicalEntries, `${run.id} active entry versions`); duplicate(logicalConnections, `${run.id} active connection versions`);
    for (const connectionId of state.connectionIds) {
      const connection = maps.connections.get(connectionId);
      if (connection) {
        const inactive = [...new Set([...connection.participants.map(item => item.entryId), ...(connection.rationaleEntryId ? [connection.rationaleEntryId] : [])].filter(item => !state.entryIds.has(item) && !(connection.relation.startsWith('legacy:') && maps.external.has(item))))];
        if (inactive.length) errors.push(`${run.id}: ${connectionId} references an inactive entry version (${inactive.join(', ')}); replace or omit the connection explicitly, including inactive rationale references`);
      }
    }
    for (const resolution of run.reviewResolutions) {
      const note = maps.notes.get(resolution.noteId);
      if (!note || !parents.some(item => item.id === note.runId)) errors.push(`${run.id}: review resolution must refer to a note on an ancestor revision`);
      if (note && Date.parse(run.createdAt) < Date.parse(note.createdAt)) errors.push(`${run.id}: review resolution predates its note`);
    }
  }
  if (bundle.runs.length) {
    const introduced = new Set(bundle.runs.flatMap(run => [...run.addedEntryIds, ...run.addedConnectionIds]));
    for (const record of [...bundle.entries, ...bundle.connections]) if (!introduced.has(record.id)) errors.push(`${record.id}: record is not introduced by an extraction run`);
  } else if (bundle.entries.length || bundle.connections.length || bundle.reviewNotes.length) errors.push('Reasoning records require an extraction run');
  for (const note of bundle.reviewNotes) {
    exists(maps.actors, note.reviewer, note.id); exists(maps.runs, note.runId, note.id); checkSpans(note.sourceSpans, note.id);
    const set = note.target.kind === 'entry' ? maps.entries : note.target.kind === 'connection' ? maps.connections : note.target.kind === 'source' ? maps.sources : null;
    if (set && !note.target.id) errors.push(`${note.id}: a ${note.target.kind} review needs a target ID`);
    if (set) exists(set, note.target.id, note.id);
    if (note.target.kind === 'missing' && note.target.id !== null) errors.push(`${note.id}: missing-contribution reviews must have a null target ID`);
    if (maps.runs.has(note.runId) && note.target.id && ['entry', 'connection'].includes(note.target.kind)) {
      try { const state = accumulated(bundle, note.runId); if (!(note.target.kind === 'entry' ? state.entryIds : state.connectionIds).has(note.target.id)) errors.push(`${note.id}: target is not active in the reviewed revision`); } catch { /* lineage error already reported */ }
    }
  }
  for (const reference of bundle.externalEntryRefs) {
    if (reference.namespace.workspaceId !== bundle.namespace.workspaceId) errors.push(`${reference.id}: cross-workspace entry references are prohibited`);
    if (reference.namespace.projectId === null || reference.namespace.datasetId === bundle.id) errors.push(`${reference.id}: external entry requires another hosted dataset`);
  }
  for (const grant of bundle.authorityGrants) {
    exists(maps.actors, grant.agentActorId, grant.id); exists(maps.actors, grant.ownerActorId, grant.id); exists(maps.grants, grant.priorGrantId, grant.id);
    if (maps.actors.get(grant.agentActorId)?.kind !== 'agent' || maps.actors.get(grant.ownerActorId)?.kind !== 'human') errors.push(`${grant.id}: grants require a human owner and a named agent`);
    if (bundle.namespace.projectId !== null && grant.projectId !== bundle.namespace.projectId) errors.push(`${grant.id}: grant belongs to another project`);
    if ((grant.action === 'grant') !== (grant.priorGrantId === null)) errors.push(`${grant.id}: revocation must reference its original grant`);
    const prior = grant.priorGrantId ? maps.grants.get(grant.priorGrantId) : undefined;
    if (prior && (prior.action !== 'grant' || prior.agentActorId !== grant.agentActorId || prior.projectId !== grant.projectId || Date.parse(prior.recordedAt) > Date.parse(grant.recordedAt))) errors.push(`${grant.id}: invalid grant revocation lineage`);
  }
  duplicate(bundle.authorityGrants.filter(grant => grant.action === 'revoke').map(grant => grant.priorGrantId!), 'Grant revocations');
  duplicate(bundle.commitmentReceipts.filter(receipt => receipt.priorReceiptId !== null).map(receipt => receipt.priorReceiptId!), 'Commitment successor receipts');
  for (const receipt of bundle.commitmentReceipts) {
    exists(maps.entries, receipt.entryId, receipt.id); exists(maps.actors, receipt.actorId, receipt.id); exists(maps.grants, receipt.grantId, receipt.id); exists(maps.receipts, receipt.priorReceiptId, receipt.id); exists(maps.connections, receipt.connectionId, receipt.id); checkSpans(receipt.sourceSpans, receipt.id);
    if (receipt.priorReceiptId === receipt.id) errors.push(`${receipt.id}: receipt cannot refer to itself`);
    if ((receipt.action === 'adopt') !== (receipt.priorReceiptId === null)) errors.push(`${receipt.id}: revision and withdrawal require the prior commitment receipt`);
    const previous = receipt.priorReceiptId ? maps.receipts.get(receipt.priorReceiptId) : undefined;
    if (previous && (previous.action === 'withdraw' || Date.parse(previous.recordedAt) > Date.parse(receipt.recordedAt))) errors.push(`${receipt.id}: invalid commitment lineage`);
    if (previous && receipt.action === 'revise' && receipt.entryId === previous.entryId) errors.push(`${receipt.id}: a revised position requires a new contribution`);
    const seen = new Set([receipt.id]); let ancestor = previous;
    while (ancestor) {
      if (seen.has(ancestor.id)) { errors.push(`${receipt.id}: commitment lineage contains a cycle`); break; }
      seen.add(ancestor.id); ancestor = ancestor.priorReceiptId ? maps.receipts.get(ancestor.priorReceiptId) : undefined;
    }
    if (receipt.basis === 'authorized_action') {
      const actor = maps.actors.get(receipt.actorId), grant = receipt.grantId ? maps.grants.get(receipt.grantId) : undefined;
      if (actor?.kind !== 'human' && actor?.kind !== 'agent') errors.push(`${receipt.id}: authorized actions require a human or delegated agent`);
      if (actor?.kind === 'human' && receipt.grantId) errors.push(`${receipt.id}: human action must not use an agent grant`);
      if (actor?.kind === 'agent' && (!grant || grant.action !== 'grant' || grant.agentActorId !== actor.id || Date.parse(grant.recordedAt) > Date.parse(receipt.recordedAt) || bundle.authorityGrants.some(event => event.action === 'revoke' && event.priorGrantId === grant.id && Date.parse(event.recordedAt) <= Date.parse(receipt.recordedAt)))) errors.push(`${receipt.id}: agent action requires its valid grant at the action time`);
    }
    if (maps.contexts.get(maps.entries.get(receipt.entryId)?.contextId ?? '')?.kind === 'illustration') errors.push(`${receipt.id}: illustration cannot become a project commitment`);
    if (receipt.basis.startsWith('legacy_') && !receipt.sourceSpans.length) errors.push(`${receipt.id}: legacy authority history requires preserved source evidence`);
    if (receipt.basis === 'legacy_state' && (!receipt.rationale.trim() || receipt.action !== 'adopt')) errors.push(`${receipt.id}: preserved legacy state requires an explanatory initial migration receipt`);
  }
  if (bundle.authorityGrants.length || bundle.commitmentReceipts.length) warnings.push('Portable authority history is untrusted on import. Only server-verified actions determine current commitments.');
  return { ok: errors.length === 0, errors: [...new Set(errors)], warnings: [...new Set(warnings)], ...(errors.length ? {} : { bundle }) };
}

function canonical(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonical).join(',')}]`;
  if (value && typeof value === 'object') return `{${Object.keys(value).sort().map(key => `${JSON.stringify(key)}:${canonical((value as Record<string, unknown>)[key])}`).join(',')}}`;
  return JSON.stringify(value);
}

/** Incremental import preserves immutable records and refuses conflicting identities. */
export function mergeBundles(previousInput: ReasoningBundle | LegacyReasoningBundle, incomingInput: ReasoningBundle | LegacyReasoningBundle): ReasoningBundle {
  const previous = upgradeBundle(previousInput), incoming = upgradeBundle(incomingInput);
  for (const bundle of [previous, incoming]) { const result = validateBundle(bundle); if (!result.ok) throw new Error(`Cannot merge invalid bundle: ${result.errors.join('; ')}`); }
  if (previous.id !== incoming.id || previous.manifest.threadId !== incoming.manifest.threadId) throw new Error('Cannot merge different conversations. Open the bundle separately.');
  if (canonical(previous.namespace) !== canonical(incoming.namespace)) throw new Error('Cannot merge different ownership namespaces');
  const merge = <T extends { id: string }>(left: T[], right: T[], label: string): T[] => {
    const result = [...left]; const byId = new Map(left.map(item => [item.id, item]));
    for (const item of right) { const existing = byId.get(item.id); if (existing && canonical(existing) !== canonical(item)) throw new Error(`Immutable ${label} conflict: ${item.id}`); if (!existing) { result.push(item); byId.set(item.id, item); } }
    return result;
  };
  const segments = previous.manifest.segments.map(segment => ({ ...segment, selectedMessageIds: [...segment.selectedMessageIds] }));
  for (const segment of incoming.manifest.segments) {
    const existing = segments.find(item => item.ref === segment.ref);
    if (existing) {
      if (existing.sha256 !== segment.sha256) throw new Error(`Frozen source segment changed: ${segment.ref}`);
      existing.selectedMessageIds = [...new Set([...existing.selectedMessageIds, ...segment.selectedMessageIds])];
    } else segments.push({ ...segment, selectedMessageIds: [...segment.selectedMessageIds] });
  }
  const merged: ReasoningBundle = {
    ...previous,
    schemaVersion: '0.3.0',
    actors: merge(previous.actors, incoming.actors, 'actor'), contexts: merge(previous.contexts, incoming.contexts, 'context'),
    sources: merge(previous.sources, incoming.sources, 'source'), attachments: merge(previous.attachments, incoming.attachments, 'attachment'),
    entries: merge(previous.entries, incoming.entries, 'entry'), connections: merge(previous.connections, incoming.connections, 'connection'),
    runs: merge(previous.runs, incoming.runs, 'run'), reviewNotes: merge(previous.reviewNotes, incoming.reviewNotes, 'review note'),
    authorityGrants: merge(previous.authorityGrants, incoming.authorityGrants, 'authority grant'), commitmentReceipts: merge(previous.commitmentReceipts, incoming.commitmentReceipts, 'commitment receipt'), externalEntryRefs: merge(previous.externalEntryRefs, incoming.externalEntryRefs, 'external entry reference'),
    manifest: { ...previous.manifest, frozenAt: Date.parse(previous.manifest.frozenAt) >= Date.parse(incoming.manifest.frozenAt) ? previous.manifest.frozenAt : incoming.manifest.frozenAt, cutoffDescription: [...new Set([...previous.manifest.cutoffDescription.split('\nIncrement: '), ...incoming.manifest.cutoffDescription.split('\nIncrement: ')])].join('\nIncrement: '), segments, exclusions: [...new Set([...previous.manifest.exclusions, ...incoming.manifest.exclusions])] },
  };
  const result = validateBundle(merged);
  if (!result.ok) throw new Error(`Merged bundle is invalid: ${result.errors.join('; ')}`);
  return result.bundle!;
}
