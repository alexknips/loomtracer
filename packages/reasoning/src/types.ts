/** Portable source-preserving research format. Offsets are UTF-16 [start,end). */
export type Actor = { id: string; name: string; kind: 'human' | 'agent' | 'system' | 'import' | 'unknown' };
export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };
export type Namespace = { datasetId: string; workspaceId: string | null; projectId: string | null };
export type SourceProvenance = {
  kind: 'chat' | 'legacy_record' | 'legacy_audit' | 'authored';
  originalRecord?: { table: string; id: string; workspaceId: string };
  recordedBy?: string | null; recordedAt?: string | null;
  payload?: Json;
};
export type Span = { messageId: string; start: number; end: number };
export type SourceMessage = {
  id: string; originalId: string; actorId: string; role: 'user' | 'assistant' | 'record' | 'audit';
  content: string; occurredAt: string | null; loggedAt: string | null; order: number;
  sourceRef: string; attachmentIds: string[];
  // Preserve alternative/resubmitted source occurrences without treating them as active turns.
  active: boolean; supersededBy?: string; provenance?: SourceProvenance;
};
export type Attachment = { id: string; mimeType: 'image/png' | 'image/jpeg' | 'image/webp'; dataUrl: string; sha256: string };
export type Context = { id: string; title: string; kind: 'discussion' | 'illustration' | 'quoted_source' | 'legacy_record' | 'ledger_record' };
export type Provenance = {
  expressedBy: string | null; recordedBy: string; occurredAt: string | null;
  recordedAt: string; basis: 'direct' | 'inferred'; sourceSpans: Span[];
};
export type Entry = {
  id: string; logicalId: string; version: number; contextId: string;
  // Prefer exact source spans. Authored content is only for a new contribution.
  content?: string; summary?: string; labels: string[]; provenance: Provenance;
};
export type NativeRelation = 'responds_to' | 'answers' | 'supports' | 'challenges' | 'compares' | 'adopts' | 'revises' | 'withdraws';
export type Relation = NativeRelation | `legacy:${string}`;
export type Participant = { entryId: string; role: string };
export type Connection = {
  id: string; logicalId: string; version: number; contextId: string; relation: Relation;
  participants: Participant[]; rationaleEntryId?: string; provenance: Provenance;
};
export type ExtractionRun = {
  id: string; parentId: string | null; createdAt: string; recordedBy: string;
  sourceCutoff: number; sourceMessageIds: string[];
  addedEntryIds: string[]; addedConnectionIds: string[];
  replacements: { kind: 'entry' | 'connection'; previousId: string; nextId: string }[];
  // Format 0.2.0: extraction corrections only, never a source-level withdrawal.
  // Removed versions remain stored and available in earlier extraction revisions.
  omissions?: { kind: 'entry' | 'connection'; recordId: string; reason: string }[];
  // Complete current uncertainty snapshot at this run's source cutoff.
  uncertainties: string[]; notes: string;
  reviewResolutions: { noteId: string; status: 'addressed' | 'unresolved'; comment: string }[];
};
export type ReviewNote = {
  id: string; reviewer: string; createdAt: string; runId: string;
  target: { kind: 'entry' | 'connection' | 'source' | 'missing'; id: string | null };
  sourceSpans: Span[]; comment: string;
};
export type SourceManifest = {
  threadId: string; frozenAt: string; cutoffDescription: string;
  segments: { ref: string; sha256: string; selectedMessageIds: string[] }[];
  exclusions: string[];
};
/** These portable records document history, not authorization. Only verified server actions confer authority. */
export type AuthorityGrant = {
  id: string; action: 'grant' | 'revoke'; agentActorId: string; ownerActorId: string;
  projectId: string; recordedAt: string; priorGrantId: string | null;
};
export type CommitmentReceipt = {
  id: string; action: 'adopt' | 'revise' | 'withdraw'; entryId: string;
  priorReceiptId: string | null; actorId: string; grantId: string | null;
  recordedAt: string; rationale: string; connectionId: string | null;
  basis: 'authorized_action' | 'legacy_approval' | 'legacy_state'; sourceSpans: Span[];
};
/** A cross-project pointer preserves an edge without copying the other project's private entry. */
export type ExternalEntryRef = { id: string; namespace: Namespace; entryId: string; originalId: string };
export type ReasoningBundle = {
  schemaVersion: '0.3.0'; id: string; title: string; namespace: Namespace;
  actors: Actor[]; contexts: Context[]; sources: SourceMessage[]; attachments: Attachment[];
  manifest: SourceManifest; entries: Entry[]; connections: Connection[];
  runs: ExtractionRun[]; reviewNotes: ReviewNote[];
  authorityGrants: AuthorityGrant[]; commitmentReceipts: CommitmentReceipt[];
  externalEntryRefs: ExternalEntryRef[];
};
export type LegacyReasoningBundle = Omit<ReasoningBundle, 'schemaVersion' | 'namespace' | 'authorityGrants' | 'commitmentReceipts' | 'externalEntryRefs'> & { schemaVersion: '0.1.0' | '0.2.0' };
export type View = { entries: Entry[]; connections: Connection[]; externalConnections?: Connection[]; run: ExtractionRun; cutoff: number };
export type ValidationResult = { ok: boolean; errors: string[]; warnings: string[]; bundle?: ReasoningBundle };
