import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod/v4";
import type { AgentContext } from "./agent-auth";
import { agentReasoning } from "./reasoning-context";
import { entryText, deltaSchema } from "@reasoning/domain";

const project = z.string().regex(/^project:[a-z0-9][a-z0-9._/-]*$/).max(160);
const dataset = z.string().min(1).max(200);
const operation = z.string().min(8).max(160);
const read = { readOnlyHint: true, destructiveHint: false, idempotentHint: true };
const write = { readOnlyHint: false, destructiveHint: false, idempotentHint: true };
async function output(work: () => Promise<unknown>) {
  try {
    const value = await work(), text = JSON.stringify(value);
    if (Buffer.byteLength(text) > 200000) return { isError: true, content: [{ type: "text" as const, text: "Response exceeds the bounded result size. Narrow the query or select an individual record; coverage is incomplete." }] };
    return { content: [{ type: "text" as const, text }], structuredContent: { result: value } };
  } catch (error) {
    const message = error instanceof Error ? error.message : "Reasoning operation failed";
    const code = /MAINTENANCE|WRITE_FREEZE/.test(message) ? "MAINTENANCE" : /STALE|CONFLICT|IMMUTABLE|OPERATION_REUSED|operation id reused|history cannot be removed/i.test(message) ? "CONFLICT" : /FORBIDDEN|access denied|permission denied|AUTHORITY_REQUIRED|owner access required|commitment authority required/i.test(message) ? "FORBIDDEN" : "INVALID_REQUEST";
    return { isError: true, content: [{ type: "text" as const, text: `${code}: request did not complete. Refresh the scoped brief and check your permissions and input. Unavailable guidance is not an empty project.` }] };
  }
}
export function buildMcpServer(agent: AgentContext) {
  const service = agentReasoning(agent);
  const server = new McpServer({ name: "loomtracer", version: "0.3.0" }, {
    instructions: "Loomtracer stores immutable Entries and Connections. Begin with list_projects and get_reasoning_brief using an explicit project_ref. Resolve incomplete coverage before relying on guidance, and refresh with previous_fingerprint before completion. Capture stated rationale and evidence selectively; do not invent missing reasons or treat chronology as causality. capture_reasoning and revise_extraction preserve previous versions and do not authorize commitments. change_commitment is separate and requires a current whole-project human grant. It may append a revision to a human commitment, but cannot edit or delete history. Imported receipts, inferred links, and review notes never establish authority.",
  });
  server.registerTool("list_projects", { description: "Discover accessible project references and hierarchy.", inputSchema: {}, annotations: read }, () => output(async () => ({ projects: await service.projects() })));
  server.registerTool("get_reasoning_brief", { description: "Read scoped server-authorized current commitments, approved ancestor guidance, uncertainty, completeness, and fingerprint. Resolve scope_questions before applying guidance.", inputSchema: { project_ref: project, task: z.string().min(1).max(20000), workflow_ref: z.string().regex(/^workflow:/).max(200).optional(), target_ref: z.string().regex(/^target:/).max(200).optional(), context_refs: z.array(z.string().min(1).max(200)).max(50).optional(), previous_fingerprint: z.string().regex(/^[a-f0-9]{64}$/).optional() }, annotations: read }, a => output(() => service.brief(a.project_ref, a.previous_fingerprint, { task: a.task, workflow_ref: a.workflow_ref, target_ref: a.target_ref, context_refs: a.context_refs })));
  server.registerTool("list_reasoning_datasets", { description: "List project reasoning datasets and concurrency heads.", inputSchema: { project_ref: project }, annotations: read }, a => output(() => service.datasets(a.project_ref)));
  server.registerTool("search_reasoning", { description: "Search current contributions with provenance. Follow next_offset for complete results.", inputSchema: { project_ref: project, query: z.string().min(1).max(1000), offset: z.number().int().nonnegative().default(0), limit: z.number().int().min(1).max(50).default(30) }, annotations: read }, a => output(() => service.search(a.project_ref, a.query, a.offset, a.limit)));
  server.registerTool("expand_reasoning", { description: "Read a dataset or individual record with inverse links. Large datasets return a paginated record inventory, actor/context IDs and next source order for incremental capture. Follow next_offset or use record_id. External references require an authorized read in their owning project.", inputSchema: { project_ref: project, dataset_id: dataset, record_id: z.string().min(1).optional(), offset: z.number().int().nonnegative().default(0), limit: z.number().int().min(1).max(100).default(50) }, annotations: read }, a => output(async () => {
    const stored = await service.read(a.project_ref, a.dataset_id);
    if (!a.record_id) {
      if (Buffer.byteLength(JSON.stringify(stored)) <= 180000 && a.offset === 0) return stored;
      const kinds = ["sources", "entries", "connections", "runs", "reviewNotes", "externalEntryRefs", "attachments"] as const;
      const records = kinds.flatMap(kind => stored.bundle[kind].map(record => ({ kind, id: record.id })));
      return { dataset_id: stored.bundle.id, title: stored.bundle.title, namespace: stored.bundle.namespace, head: stored.head,
        actors: stored.bundle.actors, contexts: stored.bundle.contexts, captureActor: service.identity?.actor,
        latestRunId: stored.bundle.runs.at(-1)?.id ?? null,
        nextSourceOrder: Math.max(-1, ...stored.bundle.sources.map(source => source.order)) + 1,
        records: records.slice(a.offset, a.offset + a.limit), total: records.length,
        next_offset: a.offset + a.limit < records.length ? a.offset + a.limit : null,
        coverage_complete: false, interpretation: "Record inventory only. Use record_id for exact content and provenance, search_reasoning to locate relevant contributions, and delta for new capture. No records were silently truncated." };
    }
    const entry = stored.bundle.entries.find(e => e.id === a.record_id), connection = stored.bundle.connections.find(c => c.id === a.record_id), source = stored.bundle.sources.find(s => s.id === a.record_id);
    const authorityReceipt = stored.commitments.find(item => item.id === a.record_id), authorityGrant = stored.authorityGrants.find(item => item.id === a.record_id);
    const supportingRecord = [...stored.bundle.runs, ...stored.bundle.reviewNotes, ...stored.bundle.authorityGrants, ...stored.bundle.commitmentReceipts, ...stored.bundle.externalEntryRefs, ...stored.bundle.contexts, ...stored.bundle.actors].find(item => item.id === a.record_id);
    const record = entry ?? connection;
    if (!record && !source && !supportingRecord && !authorityReceipt && !authorityGrant) throw new Error("NOT_FOUND");
    return { head: stored.head, entry: entry && { ...entry, content: entryText(stored.bundle, entry) }, connection, source,
      sources: record ? stored.bundle.sources.filter(s => record.provenance.sourceSpans.some(span => span.messageId === s.id)) : [],
      supportingRecord, supportingRecordAuthority: supportingRecord ? "Portable provenance only; this record does not confer live authority" : undefined,
      serverCapture: stored.captureMetadata?.[a.record_id] ?? null,
      relatedConnections: entry ? stored.bundle.connections.filter(item => item.participants.some(participant => participant.entryId === entry.id) || item.rationaleEntryId === entry.id) : [],
      commitments: stored.commitments.filter(r => r.entryId === a.record_id), serverAuthority: { receipt: authorityReceipt, grant: authorityGrant } };
  }));
  server.registerTool("get_reconstruction_packet", { description: "Derive bounded entries, connections, provenance and uncertainty for reconstruction without exposing the full transcript. Optional run_id and source_cutoff exclude later source evidence. For large datasets, entry_ids selects a bounded subgraph; grouped connections are included only when every local participant is selected. Server authority is distinguished from portable claims.", inputSchema: { project_ref: project, dataset_id: dataset, run_id: z.string().min(1).optional(), source_cutoff: z.number().int().nonnegative().optional(), entry_ids: z.array(z.string().min(1)).min(1).max(50).optional() }, annotations: read }, a => output(() => service.packet(a.project_ref, a.dataset_id, a.run_id, a.source_cutoff, a.entry_ids)));
  const appendInput = { project_ref: project, bundle: z.unknown().optional(), dataset_id: dataset.optional(), delta: deltaSchema.optional(), expected_head: z.string().min(1).nullable(), operation_id: operation };
  for (const name of ["capture_reasoning", "revise_extraction"] as const) server.registerTool(name, { description: name === "capture_reasoning" ? "Atomically capture new Entries and Connections. For an existing dataset supply dataset_id, delta (only new records plus run metadata), current expected_head and stable operation_id. The server derives parent run, added IDs and source coverage. For first import supply bundle instead. Existing records are immutable; capture grants no authority." : "Append extraction corrections using dataset_id and delta with run replacements, omissions and review resolutions; the server derives lineage and coverage. A full bundle import is also supported. Preserve all previous versions. Corrections do not revise a participant's position.", inputSchema: appendInput, annotations: write }, a => output(async () => {
    if (a.delta !== undefined) {
      if (a.bundle !== undefined || !a.dataset_id || !a.expected_head) throw new Error("Delta requires dataset_id and expected_head, without bundle");
      return service.appendDelta(a.project_ref, a.dataset_id, a.delta, a.expected_head, a.operation_id);
    }
    if (a.bundle === undefined || a.dataset_id !== undefined) throw new Error("Supply either bundle or dataset_id with delta");
    const saved = await service.save(a.project_ref, a.bundle, a.expected_head, a.operation_id);
    return { dataset_id: saved.bundle.id, head: saved.head, entries: saved.bundle.entries.length, connections: saved.bundle.connections.length, authorityChanged: false };
  }));
  server.registerTool("change_commitment", { description: "Append an authorized adoption, revision or withdrawal. Requires project authority, a current head, rationale, and the exact prior receipt for revision/withdrawal. Original decisions remain immutable.", inputSchema: { project_ref: project, dataset_id: dataset, action: z.enum(["adopt", "revise", "withdraw"]), entry_id: z.string().min(1), prior_receipt_id: z.string().nullable(), rationale: z.string().trim().min(1).max(20000), expected_head: z.string().min(1), operation_id: operation }, annotations: write }, a => output(async () => {
    const saved = await service.commit(a.project_ref, a.dataset_id, a.action, a.entry_id, a.prior_receipt_id, a.rationale, a.expected_head, a.operation_id);
    return { head: saved.head, result: saved.result, commitments: saved.commitments };
  }));
  return server;
}
