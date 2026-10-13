'use client';

import { useEffect, useMemo, useRef, useState } from 'react';
import Link from 'next/link';
import { entryText, getView, validateBundle, type Actor, type Entry, type ReasoningBundle, type Connection, type CommitmentReceipt, type AuthorityGrant } from '@reasoning';

export type WorkspaceActor = Actor;
type Project = { ref: string; name: string };
type Dataset = { id: string; title: string; head: string | null };
type AgentConnection = { id: string; name?: string; label?: string; actorId?: string; actor_id?: string; display_name?: string; revoked_at?: string | null };
export type CaptureMetadata = Record<string, { actorId: string; recordedAt: string }>;
export type HostedState = { projectRef: string; datasetId: string; head: string | null; commitments: CommitmentReceipt[]; authorityGrants: AuthorityGrant[]; captureMetadata: CaptureMetadata };
type Payload = { error?: string; bundle?: ReasoningBundle; head?: string | null; projects?: Project[]; datasets?: Dataset[]; actor?: WorkspaceActor; isOwner?: boolean; commitments?: CommitmentReceipt[]; authorityGrants?: AuthorityGrant[]; grants?: AuthorityGrant[]; connections?: AgentConnection[]; captureMetadata?: CaptureMetadata };

async function request(url: string, init?: RequestInit): Promise<Payload> {
  const response = await fetch(url, { ...init, headers: { 'Content-Type': 'application/json', ...init?.headers }, cache: 'no-store' });
  const result = await response.json().catch(() => ({})) as Payload;
  if (!response.ok) throw new Error(response.status === 401 ? 'Sign in to save this bundle to a project. The imported file remains in your browser.' : response.status === 409 ? 'This project changed since you opened it. Export your changes, then reopen the saved history before retrying.' : result.error ?? `The server returned ${response.status}. Your local changes are preserved.`);
  return result;
}
const query = (projectRef: string, datasetId?: string) => `/api/reasoning?${new URLSearchParams({ project_ref: projectRef, ...(datasetId ? { dataset_id: datasetId } : {}) })}`;

/** Append one independently authored contribution. Its source text is stored once. */
export async function appendContribution(bundle: ReasoningBundle | null, actor: Actor, title: string, text: string, label: string, target: Entry | undefined, relation: 'responds_to' | 'answers' | 'supports' | 'challenges'): Promise<ReasoningBundle> {
  const now = new Date().toISOString(), token = crypto.randomUUID();
  const datasetId = bundle?.id ?? `dataset:${token}`, sourceId = `source:${token}`, logicalId = `entry:${token}`, runId = `capture:${token}`;
  const contextId = target?.contextId ?? bundle?.contexts.find(context => context.kind === 'discussion')?.id ?? `context:${token}`;
  const order = Math.max(0, ...(bundle?.sources.map(source => source.order) ?? [])) + 1;
  const provenance = { expressedBy: actor.id, recordedBy: actor.id, occurredAt: now, recordedAt: now, basis: 'direct' as const, sourceSpans: [{ messageId: sourceId, start: 0, end: text.length }] };
  const entry: Entry = { id: `${logicalId}:v1`, logicalId, version: 1, contextId, labels: label ? [label] : [], provenance };
  const roles = { responds_to: ['response', 'target'], answers: ['answer', 'question'], supports: ['premise', 'conclusion'], challenges: ['objection', 'target'] } as const;
  const connections: Connection[] = target ? [{ id: `connection:${token}:v1`, logicalId: `connection:${token}`, version: 1, contextId, relation, participants: [{ entryId: entry.id, role: roles[relation][0] }, { entryId: target.id, role: roles[relation][1] }], provenance }] : [];
  const sha256 = [...new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text)))].map(byte => byte.toString(16).padStart(2, '0')).join('');
  const source = { id: sourceId, originalId: sourceId, actorId: actor.id, role: 'user' as const, content: text, occurredAt: now, loggedAt: now, order, sourceRef: `authored://${datasetId}/${sourceId}`, attachmentIds: [], active: true, provenance: { kind: 'authored' as const } };
  const initial: ReasoningBundle = bundle ?? { schemaVersion: '0.3.0', id: datasetId, title, namespace: { datasetId, workspaceId: null, projectId: null }, actors: [], contexts: [], sources: [], attachments: [], manifest: { threadId: datasetId, frozenAt: now, cutoffDescription: 'Contributions authored in Loomtracer.', segments: [], exclusions: [] }, entries: [], connections: [], runs: [], reviewNotes: [], authorityGrants: [], commitmentReceipts: [], externalEntryRefs: [] };
  return { ...initial, actors: initial.actors.some(item => item.id === actor.id) ? initial.actors : [...initial.actors, actor], contexts: initial.contexts.some(item => item.id === contextId) ? initial.contexts : [...initial.contexts, { id: contextId, title: title || initial.title, kind: 'discussion' }], sources: [...initial.sources, source], entries: [...initial.entries, entry], connections: [...initial.connections, ...connections], manifest: { ...initial.manifest, segments: [...initial.manifest.segments, { ref: source.sourceRef, sha256, selectedMessageIds: [sourceId] }] }, runs: [...initial.runs, { id: runId, parentId: initial.runs.at(-1)?.id ?? null, createdAt: now, recordedBy: actor.id, sourceCutoff: order, sourceMessageIds: [sourceId], addedEntryIds: [entry.id], addedConnectionIds: connections.map(item => item.id), replacements: [], uncertainties: initial.runs.at(-1)?.uncertainties ?? [], notes: 'A new contribution was authored directly in Loomtracer. Earlier contributions are unchanged.', reviewResolutions: [] }] };
}

export function HostedControls({ mode, initialProject, bundle, dirty, historical, selectedEntry, hosted, setHosted, onActor, onLoad, onInspectEntry, onError, onNotice }: {
  mode: 'local' | 'hosted'; initialProject: string; bundle: ReasoningBundle | null; dirty: boolean; historical: boolean; selectedEntry?: Entry; hosted: HostedState | null;
  setHosted: (state: HostedState | null) => void; onActor: (actor: WorkspaceActor) => void; onLoad: (bundle: ReasoningBundle, message: string, preserveView?: boolean) => boolean; onError: (message: string) => void; onNotice: (message: string) => void; onInspectEntry: (id: string) => void;
}) {
  const [connected, setConnected] = useState(mode === 'hosted');
  const [actor, setActor] = useState<Actor | null>(null), [isOwner, setIsOwner] = useState(false);
  const [projects, setProjects] = useState<Project[]>([]), [projectRef, setProjectRef] = useState(initialProject), [datasets, setDatasets] = useState<Dataset[]>([]);
  const [grants, setGrants] = useState<AuthorityGrant[]>([]), [agents, setAgents] = useState<AgentConnection[]>([]);
  const [busy, setBusy] = useState(false), [composerOpen, setComposerOpen] = useState(false), [text, setText] = useState(''), [title, setTitle] = useState('New reasoning history'), [label, setLabel] = useState('');
  const [linkEntryId, setLinkEntryId] = useState(''), [relation, setRelation] = useState<'responds_to' | 'answers' | 'supports' | 'challenges'>('responds_to');
  const [action, setAction] = useState<'adopt' | 'revise' | 'withdraw'>('adopt'), [priorReceipt, setPriorReceipt] = useState(''), [rationale, setRationale] = useState(''), [agentId, setAgentId] = useState('');
  const callbacks = useRef({ onActor, onError, onLoad, onNotice, setHosted, hasBundle: Boolean(bundle) });
  useEffect(() => { callbacks.current = { onActor, onError, onLoad, onNotice, setHosted, hasBundle: Boolean(bundle) }; }, [onActor, onError, onLoad, onNotice, setHosted, bundle]);
  const initialLoadAttempted = useRef(false);
  const captureDraft = useRef<{ fingerprint: string; bundle: ReasoningBundle } | null>(null);
  const pendingOperation = useRef<{ fingerprint: string; id: string } | null>(null);
  const entries = useMemo(() => bundle?.runs.length ? getView(bundle).entries : [], [bundle]);
  const revoked = new Set(grants.filter(grant => grant.action === 'revoke').map(grant => grant.priorGrantId));
  const activeGrants = grants.filter(grant => grant.action === 'grant' && !revoked.has(grant.id));
  const selectedContext = bundle?.contexts.find(context => context.id === selectedEntry?.contextId)?.kind;
  const selectedIsIllustration = selectedContext === 'illustration' || selectedContext === 'quoted_source';
  const receipts = hosted?.commitments ?? [];
  const superseded = new Set(receipts.map(receipt => receipt.priorReceiptId).filter(Boolean));
  const current = receipts.filter(receipt => receipt.action !== 'withdraw' && !superseded.has(receipt.id));

  useEffect(() => {
    if (!connected) return;
    const controller = new AbortController();
    void request('/api/reasoning', { signal: controller.signal }).then(data => {
      setProjects(data.projects ?? []); setIsOwner(data.isOwner ?? false);
      if (data.actor) { setActor(data.actor); callbacks.current.onActor(data.actor); }
      if (mode === 'hosted') setProjectRef(ref => data.projects?.some(project => project.ref === ref) ? ref : data.projects?.[0]?.ref || '');
    }).catch(error => { if (!controller.signal.aborted) callbacks.current.onError(error.message); });
    return () => controller.abort();
  }, [connected, mode]);
  useEffect(() => {
    if (!connected || !projectRef) return;
    const controller = new AbortController();
    void request(query(projectRef), { signal: controller.signal }).then(async data => { setDatasets(data.datasets ?? []); setGrants(data.grants ?? []); setAgents(data.connections ?? []); if (mode === 'hosted' && !initialLoadAttempted.current && data.datasets?.length) { initialLoadAttempted.current = true; const detail = await request(query(projectRef, data.datasets[0].id), { signal: controller.signal }); if (!controller.signal.aborted && detail.bundle && !callbacks.current.hasBundle && callbacks.current.onLoad(detail.bundle, 'Project history opened.', false)) { callbacks.current.setHosted({ projectRef, datasetId: detail.bundle.id, head: detail.head ?? null, commitments: detail.commitments ?? [], authorityGrants: detail.authorityGrants ?? [], captureMetadata: detail.captureMetadata ?? {} }); } } }).catch(error => { if (!controller.signal.aborted) callbacks.current.onError(error.message); });
    return () => controller.abort();
  }, [connected, projectRef, mode]);

  function getOperation(payload: unknown) {
    const fingerprint = JSON.stringify(payload);
    if (pendingOperation.current?.fingerprint !== fingerprint) pendingOperation.current = { fingerprint, id: crypto.randomUUID() };
    return pendingOperation.current.id;
  }
  async function refreshProject() {
    const data = await request(query(projectRef)); setDatasets(data.datasets ?? []); setGrants(data.grants ?? []); setAgents(data.connections ?? []);
  }
  function accept(data: Payload, nextBundle?: ReasoningBundle, preserveView = true) {
    const result = validateBundle(data.bundle ?? nextBundle);
    if (!result.ok || !result.bundle) throw new Error(result.errors.join('; '));
    if (!callbacks.current.onLoad(result.bundle, 'Saved to your project. Earlier records remain inspectable.', preserveView)) return;
    callbacks.current.setHosted({ projectRef, datasetId: result.bundle.id, head: data.head ?? null, commitments: data.commitments ?? [], authorityGrants: data.authorityGrants ?? [], captureMetadata: data.captureMetadata ?? {} });
  }
  async function task(work: () => Promise<void>) {
    setBusy(true); try { await work(); } catch (error) { onError(error instanceof Error ? error.message : 'The operation failed. Your local data is preserved.'); } finally { setBusy(false); }
  }
  async function openDataset(id: string) {
    if (dirty && !window.confirm('Export or save your changes before opening another history. Discard unsaved changes?')) return;
    await task(async () => { const data = await request(query(projectRef, id)); accept(data, undefined, false); });
  }
  async function save(nextBundle: ReasoningBundle) {
    if (!projectRef || !actor) return;
    const payload = { project_ref: projectRef, bundle: nextBundle, expected_head: hosted?.projectRef === projectRef && hosted.datasetId === nextBundle.id ? hosted.head : null };
    const data = await request('/api/reasoning', { method: 'POST', body: JSON.stringify({ ...payload, operation_id: getOperation(payload) }) });
    accept(data, nextBundle); pendingOperation.current = null; await refreshProject();
  }
  async function capture() {
    if (!actor || !text.trim()) return;
    const fingerprint = JSON.stringify({ datasetId: hosted?.datasetId, head: hosted?.head, projectRef, title, text, label, linkEntryId, relation });
    if (captureDraft.current?.fingerprint !== fingerprint) captureDraft.current = { fingerprint, bundle: await appendContribution(hosted?.projectRef === projectRef ? bundle : null, actor, title, text.trim(), label, entries.find(entry => entry.id === linkEntryId), relation) };
    const next = captureDraft.current.bundle;
    const result = validateBundle(next); if (!result.ok) throw new Error(result.errors.join('; '));
    await save(next); captureDraft.current = null; setText(''); setComposerOpen(false); setLinkEntryId('');
  }
  async function commitment() {
    if (!hosted || !rationale.trim()) return;
    const prior = current.find(item => item.id === priorReceipt);
    const payload = { project_ref: hosted.projectRef, dataset_id: hosted.datasetId, action, entry_id: action === 'withdraw' ? prior?.entryId : selectedEntry?.id, prior_receipt_id: action === 'adopt' ? null : priorReceipt, rationale: rationale.trim(), expected_head: hosted.head };
    const data = await request('/api/reasoning/commitments', { method: 'POST', body: JSON.stringify({ ...payload, operation_id: getOperation(payload) }) });
    accept(data); pendingOperation.current = null; setRationale(''); setPriorReceipt(''); onNotice('Commitment action recorded. The earlier decision remains in history.');
  }
  const actorName = (id: string) => id === actor?.id ? actor.name : agents.find(item => { const actorId = item.actorId ?? item.actor_id; return actorId === id || (actorId && `agent:${actorId}` === id); })?.display_name ?? id;

  return <section className="hosted-controls" aria-label="Project history">
    <div className="hosted-controls-top"><div><p className="eyebrow">{mode === 'local' ? 'Private inspection' : 'Your project'}</p><p className="muted">{mode === 'local' ? 'Opening a file does not upload it. Saving to a project is a separate action.' : 'Capture contributions, follow their connections, and retain every decision.'}</p></div><Link className="text-button" href={mode === 'local' ? '/graph' : '/inspect'}>{mode === 'local' ? 'Open your projects →' : 'Inspect a private file →'}</Link></div>
    {!connected ? <button className="button secondary" onClick={() => setConnected(true)}>Connect to a project to save</button> : <div className="project-controls">
      <label>Project<select aria-label="Reasoning project" value={projectRef} disabled={busy} onChange={event => { setHosted(null); setProjectRef(event.target.value); setDatasets([]); setGrants([]); setAgents([]); }}><option value="">Choose a project</option>{projects.map(project => <option key={project.ref} value={project.ref}>{project.name}</option>)}</select></label>
      <label>Saved history<select aria-label="Saved reasoning history" value={hosted?.projectRef === projectRef ? hosted.datasetId : ''} disabled={busy || !projectRef} onChange={event => { if (event.target.value) void openDataset(event.target.value); }}><option value="">{datasets.length ? 'Choose a history' : 'No saved history'}</option>{datasets.map(dataset => <option key={dataset.id} value={dataset.id}>{dataset.title}</option>)}</select></label>
      {hosted && <button className="text-button" disabled={busy} onClick={() => { void openDataset(hosted.datasetId); }}>Reload saved history</button>}
      {bundle && <button className="button primary" data-testid="save-project" disabled={busy || !projectRef || !actor} onClick={() => { void task(() => save(bundle)); }}>{hosted?.projectRef === projectRef ? 'Save changes' : 'Save to project'}</button>}
      <button className="button secondary" disabled={busy || !projectRef || !actor} onClick={() => setComposerOpen(open => !open)}>{composerOpen ? 'Close capture' : '+ Capture contribution'}</button>
      {mode === 'hosted' && dirty && <span className="unsaved" role="status" data-testid="review-unsaved"><span />Unsaved changes</span>}
      {busy && <span role="status">Saving or loading…</span>}
    </div>}
    {connected && !actor && <p className="muted"><Link href="/login" className="text-button">Sign in</Link> to use your hosted projects. Local inspection remains available.</p>}
    {composerOpen && <form className="capture-form" onSubmit={event => { event.preventDefault(); void task(capture); }}>
      {!hosted && <label>History title<input aria-label="History title" value={title} onChange={event => setTitle(event.target.value)} required /></label>}
      <label>Contribution<textarea aria-label="New contribution" value={text} onChange={event => setText(event.target.value)} placeholder="What should survive this moment?" rows={3} required /></label>
      <div className="project-controls"><label>Retrieval label (optional)<select aria-label="Contribution label" value={label} onChange={event => setLabel(event.target.value)}><option value="">No label</option>{['question', 'proposal', 'observation', 'assumption', 'criterion', 'assessment'].map(value => <option key={value}>{value}</option>)}</select></label>
        {hosted && <><label>Connection (optional)<select aria-label="Connection target" value={linkEntryId} onChange={event => setLinkEntryId(event.target.value)}><option value="">No connection yet</option>{entries.map(entry => <option key={entry.id} value={entry.id}>{entryText(bundle!, entry).slice(0, 80)}</option>)}</select></label>{linkEntryId && <label>Relationship<select aria-label="Capture relationship" value={relation} onChange={event => setRelation(event.target.value as typeof relation)}><option value="responds_to">Responds to</option><option value="answers">Answers</option><option value="supports">Supports</option><option value="challenges">Challenges</option></select></label>}</>}
        <button className="button primary" disabled={busy || !text.trim() || (Boolean(bundle) && !hosted && dirty)} type="submit">Record contribution</button></div>
      <p className="muted">Recorded as a new contribution by {actor?.name}. This does not adopt a commitment.</p>
    </form>}
    {hosted && historical && <p className="muted">Current commitment actions are withheld in this historical view. Return to the latest revision and full conversation cutoff to inspect authority.</p>}
    {hosted && !historical && <details className="authority-panel"><summary>Commitments and authority <span>{current.length} current commitments</span></summary><p>Only server-verified actions establish commitments. Imported adoption links and review notes do not grant authority.</p>
      {current.length ? current.map(receipt => <article className="commitment-card" key={receipt.id}><strong>{bundle?.entries.find(entry => entry.id === receipt.entryId) ? entryText(bundle, bundle.entries.find(entry => entry.id === receipt.entryId)!) : receipt.entryId}</strong><p>{receipt.rationale}</p><small>By {actorName(receipt.actorId)} · {new Date(receipt.recordedAt).toLocaleString()} · {receipt.basis === 'legacy_state' ? 'Preserved legacy state · original approver not established' : receipt.basis === 'legacy_approval' ? 'Preserved historical approval' : receipt.grantId ? `Agent grant ${receipt.grantId}` : 'Human action'}</small><code>{receipt.id}</code><button type="button" className="text-button" onClick={() => onInspectEntry(receipt.entryId)}>Inspect original contribution →</button></article>) : <p className="muted">No current commitment is established for this history.</p>}
      {actor && <form className="commitment-form" onSubmit={event => { event.preventDefault(); void task(commitment); }}><h3>Record a commitment action</h3><p className="muted">Select a contribution to adopt or use as a revision. The original record always remains.</p><div className="project-controls"><label>Action<select value={action} aria-label="Commitment action" onChange={event => setAction(event.target.value as typeof action)}><option value="adopt">Adopt selected contribution</option><option value="revise">Revise an existing commitment</option>{isOwner && <option value="withdraw">Withdraw an existing commitment</option>}</select></label>{action !== 'adopt' && <label>Prior commitment<select aria-label="Prior commitment" required value={priorReceipt} onChange={event => setPriorReceipt(event.target.value)}><option value="">Select a commitment</option>{current.map(receipt => <option value={receipt.id} key={receipt.id}>{receipt.entryId}</option>)}</select></label>}</div>{action !== 'withdraw' && <p>{selectedEntry ? `Selected: ${entryText(bundle!, selectedEntry).slice(0, 140)}` : 'Select a contribution in the timeline or graph.'}</p>}<label>Reason<textarea aria-label="Commitment rationale" value={rationale} onChange={event => setRationale(event.target.value)} required rows={2} /></label><button className="button primary" disabled={busy || dirty || !rationale.trim() || (action !== 'withdraw' && (!selectedEntry || selectedIsIllustration)) || (action !== 'adopt' && !priorReceipt)} type="submit">Record {action}</button>{selectedIsIllustration && action !== 'withdraw' && <p className="muted">Illustrative and quoted contributions cannot become project commitments.</p>}{dirty && <p className="muted">Save your pending changes before recording an authority action.</p>}</form>}
      {receipts.length > 0 && <details><summary>All {receipts.length} commitment receipts</summary>{receipts.map(receipt => <p key={receipt.id}><code>{receipt.id}</code> · {receipt.action} · {actorName(receipt.actorId)} · {receipt.entryId}{receipt.priorReceiptId && <> · follows <code>{receipt.priorReceiptId}</code></>}</p>)}</details>}
    </details>}
    {connected && projectRef && isOwner && <details className="authority-panel"><summary>Agent commitment grants <span>{activeGrants.length} active</span></summary><p>A grant permits this named agent to adopt, revise, or withdraw commitments in this project, including human decisions. It does not include descendant projects or account administration.</p>
      <form className="project-controls" onSubmit={event => { event.preventDefault(); void task(async () => { await request('/api/reasoning/grants', { method: 'POST', body: JSON.stringify({ project_ref: projectRef, connection_id: agentId }) }); setAgentId(''); await refreshProject(); }); }}><label>Agent connection<select aria-label="Agent connection for grant" value={agentId} onChange={event => setAgentId(event.target.value)} required><option value="">Choose a named agent connection</option>{agents.filter(agent => !agent.revoked_at).map(agent => <option key={agent.id} value={agent.id}>{agent.name ?? agent.display_name ?? agent.label ?? agent.id}</option>)}</select></label><button type="submit" className="button secondary" disabled={busy || !agentId}>Grant project authority</button></form>
      {activeGrants.map(grant => <div className="grant-row" key={grant.id}><span>{actorName(grant.agentActorId)} <code>{grant.id}</code></span><button className="text-button" disabled={busy} onClick={() => { void task(async () => { await request('/api/reasoning/grants', { method: 'DELETE', body: JSON.stringify({ project_ref: projectRef, grant_id: grant.id }) }); await refreshProject(); }); }}>Revoke future authority</button></div>)}
    </details>}
  </section>;
}
