import type { SupabaseClient } from "@supabase/supabase-js";
import { validateBundle, entryText, getView, buildEvaluationPacket, deltaSchema } from "@reasoning/domain";
import type { ReasoningBundle, Actor, CommitmentReceipt, AuthorityGrant } from "@reasoning/types";
import type { ProjectRecord } from "@/types/graph";
import { readWorkspaceRows } from "./workspace";
import { ancestors } from "../projects";
import { nodeApplicability, recordHash, selectedBriefScope, type BriefRequest } from "./task-brief";

export type ReasoningIdentity = { workspaceId: string; actor: Actor; isOwner: boolean; canWrite: boolean; agent: boolean };
export type DatasetSummary = { id: string; title: string; head: string };
export type CaptureMetadata = Record<string, { actorId: string; recordedAt: string }>;
export type StoredReasoning = { bundle: ReasoningBundle; head: string; commitments: CommitmentReceipt[]; authorityGrants: AuthorityGrant[]; captureMetadata?: CaptureMetadata };
export type ReasoningBriefScope = Pick<BriefRequest, "task" | "workflow_ref" | "target_ref" | "context_refs">;

/** Preserve approved legacy applicability along an authorized revision lineage. Imported labels confer none. */
function commitmentPolicy(stored: StoredReasoning, receipt: CommitmentReceipt) {
  const receipts = new Map(stored.commitments.map(item => [item.id, item]));
  const seen = new Set<string>(); let root = receipt;
  while (root.priorReceiptId) {
    if (seen.has(root.id)) throw new Error("Invalid server commitment lineage");
    seen.add(root.id); const prior = receipts.get(root.priorReceiptId);
    if (!prior) throw new Error("Incomplete server commitment lineage");
    root = prior;
  }
  const fallback = { scope_refs: [] as string[], inheritance_mode: "local" as const, legacy_source_id: null as string | null };
  if (root.basis !== "legacy_approval" && root.basis !== "legacy_state") return fallback;
  const entry = stored.bundle.entries.find(item => item.id === root.entryId);
  const source = entry?.provenance.sourceSpans.map(span => stored.bundle.sources.find(item => item.id === span.messageId)).find(item => item?.provenance?.originalRecord?.table === "nodes");
  const payload = source?.provenance?.payload;
  if (!source || !payload || typeof payload !== "object" || Array.isArray(payload)) throw new Error("Legacy approval has no preserved policy source");
  if (payload.id !== source.provenance!.originalRecord!.id || payload.workspace_id !== stored.bundle.namespace.workspaceId || payload.project_id !== stored.bundle.namespace.projectId) throw new Error("Legacy approval policy ownership is inconsistent");
  return { scope_refs: Array.isArray(payload.scope_refs) ? payload.scope_refs.filter((item): item is string => typeof item === "string") : [], inheritance_mode: payload.inheritance_mode === "descendants" ? "descendants" as const : "local" as const, legacy_source_id: source.id };
}

export class ReasoningService {
  constructor(readonly client: SupabaseClient, readonly identity: ReasoningIdentity) {}
  async rpc<T>(name: string, args: Record<string, unknown>): Promise<T> {
    const { data, error } = await this.client.rpc(name, args);
    if (error) throw new Error(error.message);
    return data as T;
  }
  async projects() { return readWorkspaceRows<ProjectRecord>(this.client, "projects", this.identity.workspaceId); }
  async readiness() {
    const context = await this.rpc<{ mode?: string }>("reasoning_context", {});
    return { mode: context?.mode ?? "unknown", ready: context?.mode === "reasoning_open" };
  }
  async project(ref: string) {
    const project = (await this.projects()).find(p => p.ref === ref);
    if (!project) throw new Error("NOT_FOUND");
    return project;
  }
  async datasets(ref: string) {
    const p = await this.project(ref);
    return this.rpc<{ datasets: DatasetSummary[]; grants: AuthorityGrant[]; commitments: CommitmentReceipt[]; connections: unknown[] }>("reasoning_list_datasets", { target_workspace_id: this.identity.workspaceId, target_project_id: p.id });
  }
  async read(ref: string, datasetId: string): Promise<StoredReasoning> {
    const p = await this.project(ref);
    const result = await this.rpc<StoredReasoning>("reasoning_read_bundle", { target_workspace_id: this.identity.workspaceId, target_project_id: p.id, target_dataset_id: datasetId });
    if (!result?.bundle) throw new Error("NOT_FOUND");
    const checked = validateBundle(result.bundle);
    if (!checked.ok || !checked.bundle) throw new Error("Stored reasoning failed validation");
    return { ...result, bundle: checked.bundle, commitments: result.commitments ?? [], authorityGrants: result.authorityGrants ?? [], captureMetadata: result.captureMetadata ?? {} };
  }
  async save(ref: string, input: unknown, expectedHead: string | null, operationId: string) {
    if (!this.identity.canWrite) throw new Error("FORBIDDEN");
    const p = await this.project(ref), checked = validateBundle(input);
    if (!checked.ok || !checked.bundle) throw new Error("Invalid bundle");
    const bundle = checked.bundle;
    if (bundle.namespace?.workspaceId && (bundle.namespace.workspaceId !== this.identity.workspaceId || bundle.namespace.projectId !== p.id)) throw new Error("FORBIDDEN");
    const hosted = { ...bundle, namespace: { datasetId: bundle.id, workspaceId: this.identity.workspaceId, projectId: p.id } };
    await this.rpc("reasoning_append_bundle", { target_workspace_id: this.identity.workspaceId, target_project_id: p.id, target_bundle: hosted, expected_head: expectedHead, operation_id: operationId });
    return this.read(ref, bundle.id);
  }
  async appendDelta(ref: string, datasetId: string, input: unknown, expectedHead: string, operationId: string) {
    if (!this.identity.canWrite) throw new Error("FORBIDDEN");
    const delta = deltaSchema.parse(input), p = await this.project(ref);
    // The database hashes the original delta before merging, so a retry stays stable
    // even when another writer has appended records after the first successful call.
    const saved = await this.rpc<{ datasetId: string; head: string }>("reasoning_append_delta", {
      target_workspace_id: this.identity.workspaceId, target_project_id: p.id,
      target_dataset_id: datasetId, target_delta: delta, expected_head: expectedHead, operation_id: operationId,
    });
    return { dataset_id: saved.datasetId, head: saved.head, authorityChanged: false as const };
  }
  async commit(ref: string, datasetId: string, action: "adopt" | "revise" | "withdraw", entryId: string, priorReceiptId: string | null, rationale: string, expectedHead: string, operationId: string) {
    if (!this.identity.canWrite) throw new Error("FORBIDDEN");
    const p = await this.project(ref);
    const result = await this.rpc("reasoning_commit", { target_workspace_id: this.identity.workspaceId, target_project_id: p.id, target_dataset_id: datasetId, action, contribution_id: entryId, prior_receipt_id: priorReceiptId, rationale, expected_head: expectedHead, operation_id: operationId });
    return { result, ...await this.read(ref, datasetId) };
  }
  async grant(ref: string, connectionId: string) {
    if (!this.identity.isOwner || this.identity.agent) throw new Error("FORBIDDEN");
    const p = await this.project(ref);
    return this.rpc("reasoning_grant_authority", { target_workspace_id: this.identity.workspaceId, target_project_id: p.id, target_connection_id: connectionId });
  }
  async revoke(ref: string, grantId: string) {
    if (!this.identity.isOwner || this.identity.agent) throw new Error("FORBIDDEN");
    const p = await this.project(ref);
    return this.rpc("reasoning_revoke_authority", { target_workspace_id: this.identity.workspaceId, target_project_id: p.id, target_grant_id: grantId });
  }
  async brief(ref: string, previousFingerprint?: string, scope: ReasoningBriefScope = { task: "Review project guidance" }) {
    const readiness = await this.readiness();
    const projects = await this.projects(), project = projects.find(item => item.ref === ref);
    if (!project) throw new Error("NOT_FOUND");
    const lineage = [project, ...ancestors(projects, project.id)];
    const datasets: { id: string; project_ref: string; inherited: boolean; head: string; commitments: CommitmentReceipt[]; uncertainties: string[]; entries: unknown[] }[] = [];
    const coverage: { project_ref: string; dataset_id: string; head: string }[] = [];
    const scopeQuestions: { entry_id: string; dataset_id: string; project_ref: string; dimension: string; options: string[] }[] = [];
    const excluded: { entry_id: string; dataset_id: string; project_ref: string; reason: string }[] = [];
    const snapshots: unknown[] = []; const warnings: string[] = [];
    if (!readiness.ready) warnings.push(`Reasoning migration is not open (mode: ${readiness.mode}). Available records are for inspection only; this is not complete authoritative guidance or clearance to proceed.`);
    for (const owner of lineage) {
      const inventory = await this.datasets(owner.ref);
      const inherited = owner.id !== project.id;
      for (const dataset of [...inventory.datasets].sort((a, b) => a.id.localeCompare(b.id))) {
        const stored = await this.read(owner.ref, dataset.id);
        coverage.push({ project_ref: owner.ref, dataset_id: dataset.id, head: stored.head });
        if (stored.head !== dataset.head) warnings.push(`Dataset ${dataset.id} changed during retrieval; refresh before applying guidance.`);
        const superseded = new Set(stored.commitments.map(r => r.priorReceiptId).filter(Boolean));
        const current = stored.commitments.filter(r => !superseded.has(r.id) && r.action !== "withdraw");
        const selected: CommitmentReceipt[] = []; const entries: unknown[] = [];
        for (const receipt of current) {
          const policy = commitmentPolicy(stored, receipt);
          if (inherited && policy.inheritance_mode !== "descendants") continue;
          const applicable = nodeApplicability({ id: receipt.entryId, scope_refs: policy.scope_refs }, scope);
          if (!applicable.applicable) {
            scopeQuestions.push(...applicable.missing.map(question => ({ entry_id: receipt.entryId, dataset_id: dataset.id, project_ref: owner.ref, dimension: question.dimension, options: question.options })));
            excluded.push({ entry_id: receipt.entryId, dataset_id: dataset.id, project_ref: owner.ref, reason: applicable.mismatch ? "Outside the selected scope" : "Scope must be resolved" });
            continue;
          }
          const entry = stored.bundle.entries.find(item => item.id === receipt.entryId);
          if (!entry) throw new Error("Authorized commitment refers to an unavailable contribution");
          selected.push(receipt); entries.push({ ...entry, content: entryText(stored.bundle, entry), ...policy, source_project_ref: owner.ref, inherited_from_project_id: inherited ? owner.id : null, serverCapture: stored.captureMetadata?.[entry.id] ?? null });
        }
        const view = stored.bundle.runs.length ? getView(stored.bundle) : null;
        // Ancestor uncertainty is context only when approved inherited guidance was selected.
        if (!inherited || selected.length || scopeQuestions.some(question => question.dataset_id === dataset.id && question.project_ref === owner.ref)) datasets.push({ id: dataset.id, project_ref: owner.ref, inherited, head: stored.head, commitments: selected, uncertainties: view?.run.uncertainties ?? [], entries });
        snapshots.push({ project_ref: owner.ref, id: dataset.id, head: stored.head, commitments: stored.commitments, authorityGrants: stored.authorityGrants, uncertainties: view?.run.uncertainties ?? [] });
      }
      const refreshed = await this.datasets(owner.ref);
      if (recordHash(inventory) !== recordHash(refreshed)) warnings.push(`Project ${owner.ref} changed during retrieval; refresh before applying guidance.`);
    }
    const refreshedProjects = await this.projects();
    if (recordHash(projects) !== recordHash(refreshedProjects)) warnings.push("Project hierarchy changed during retrieval; refresh before applying guidance.");
    const refreshedReadiness = await this.readiness();
    if (recordHash(readiness) !== recordHash(refreshedReadiness)) warnings.push("Reasoning migration readiness changed during retrieval; refresh before applying guidance.");
    const hash = recordHash({ lineage, snapshots, readiness, refreshedReadiness, selected_scope: selectedBriefScope(scope), task: scope.task, scopeQuestions });
    const result = { project_ref: ref, task: scope.task, selected_scope: selectedBriefScope(scope), readiness: refreshedReadiness, fingerprint: hash, changed: hash !== previousFingerprint, complete: !warnings.length && !scopeQuestions.length, coverage_complete: !warnings.length, datasets, scope_questions: scopeQuestions, excluded, warnings,
      interpretation: "Only server-authorized receipts establish current commitments. Approved ancestor inheritance and its original scope are preserved through authorized revisions. Native commitments default to their own project. Ancestor and local commitments remain distinct; neither silently overrides the other. Imported links and extraction reviews confer no authority. Unknown rationale is not established." };
    if (Buffer.byteLength(JSON.stringify(result)) > 180000) return { project_ref: ref, readiness: refreshedReadiness, fingerprint: hash, changed: hash !== previousFingerprint, complete: false, coverage_complete: false, datasets: coverage, scope_questions: scopeQuestions, warnings: [...warnings, "Project guidance exceeds the bounded brief. Expand every listed dataset and resolve scope before applying guidance."] };
    return result;
  }
  async search(ref: string, query: string, offset = 0, limit = 30) {
    const inventory = await this.datasets(ref), matches: unknown[] = [];
    for (const dataset of inventory.datasets) {
      const { bundle } = await this.read(ref, dataset.id);
      if (!bundle.runs.length) continue;
      for (const entry of getView(bundle).entries) {
        const content = entryText(bundle, entry);
        if (content.toLocaleLowerCase().includes(query.toLocaleLowerCase())) matches.push({ dataset_id: dataset.id, ...entry, content });
      }
    }
    return { matches: matches.slice(offset, offset + limit), total: matches.length, next_offset: offset + limit < matches.length ? offset + limit : null };
  }
  async packet(ref: string, datasetId: string, runId?: string, cutoff?: number, selectedEntryIds?: string[]) {
    const stored = await this.read(ref, datasetId);
    const packet = buildEvaluationPacket(stored.bundle, runId, cutoff);
    const totalEntries = packet.entries.length, totalConnections = packet.connections.length;
    if (selectedEntryIds) {
      const selected = new Set(selectedEntryIds), available = new Set(packet.entries.map(entry => entry.id));
      if (selectedEntryIds.some(id => !available.has(id))) throw new Error("Selected entry is unavailable at this historical boundary");
      const external = new Set(packet.externalEntryRefs.map(ref => ref.id));
      packet.entries = packet.entries.filter(entry => selected.has(entry.id));
      packet.connections = packet.connections.filter(connection => connection.participants.every(participant => selected.has(participant.entryId) || external.has(participant.entryId)) && (!connection.rationaleEntryId || selected.has(connection.rationaleEntryId)));
      packet.externalEntryRefs = packet.externalEntryRefs.filter(ref => packet.connections.some(connection => connection.participants.some(participant => participant.entryId === ref.id)));
      packet.commitmentReceipts = packet.commitmentReceipts.filter(receipt => selected.has(receipt.entryId));
      packet.authorityGrants = packet.authorityGrants.filter(grant => packet.commitmentReceipts.some(receipt => receipt.grantId === grant.id));
      const neededSources = new Set([...packet.entries, ...packet.connections, ...packet.commitmentReceipts].flatMap(record => "provenance" in record ? record.provenance.sourceSpans.map(span => span.messageId) : record.sourceSpans.map(span => span.messageId)));
      packet.sources = packet.sources.filter(source => neededSources.has(source.id));
      Object.assign(packet, { selection: { requestedEntryIds: selectedEntryIds, omittedEntries: totalEntries - packet.entries.length, omittedConnections: totalConnections - packet.connections.length, interpretation: "Selected subgraph only. Connections with omitted participants are excluded whole, never weakened into partial assertions. Dataset uncertainties remain contextual." } });
    }
    const entryIds = new Set(packet.entries.map(entry => entry.id));
    const historical = runId !== undefined || cutoff !== undefined;
    const sources = new Map(stored.bundle.sources.map(source => [source.id, source]));
    const covered = new Set(packet.sources.map(source => source.id));
    const eligibleSourceTimes = [...covered].map(id => sources.get(id)?.occurredAt).filter((time): time is string => !!time).map(time => Date.parse(time));
    const boundary = eligibleSourceTimes.length ? Math.max(...eligibleSourceTimes) : null;
    const receipts = stored.commitments.filter(receipt => entryIds.has(receipt.entryId) && (!historical || (receipt.sourceSpans.length ? receipt.sourceSpans.every(span => covered.has(span.messageId)) : boundary !== null && Date.parse(receipt.recordedAt) <= boundary)));
    const packetIds = new Set([...packet.entries.map(entry => entry.id), ...packet.connections.map(connection => connection.id), ...packet.sources.map(source => source.id)]);
    return { ...packet, head: stored.head, serverCapture: Object.fromEntries(Object.entries(stored.captureMetadata ?? {}).filter(([id]) => packetIds.has(id))), serverAuthority: { historical, receipts, authorityGrants: stored.authorityGrants.filter(grant => receipts.some(receipt => receipt.grantId === grant.id)), interpretation: historical ? "Only authority actions whose evidence or event time fits this source boundary are included. Missing historical authority is not established." : "Server-validated receipt history at this read, distinct from imported portable receipt claims." }, captureInterpretation: "serverCapture records who submitted each immutable record to hosted storage. It does not establish who originally expressed the contribution; portable provenance remains separately attributed." };
  }
}
