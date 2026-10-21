'use client';

import './reasoning.css';
import { HostedControls, type WorkspaceActor, type HostedState, type CaptureMetadata } from './HostedControls';
import { lazy, Suspense, useEffect, useMemo, useRef, useState, type ChangeEvent, type ReactNode } from 'react';
import type { Entry, Connection, Provenance, ReasoningBundle, ReviewNote, SourceMessage, Span } from '@reasoning';
import { entryText, getView, relatedConnections, validateBundle } from '@reasoning';
import { makeDemoBundle } from './fixture';
import type { Selection } from './GraphView';
const GraphView = lazy(() => import('./GraphView'));

const relationLabel = (relation: string) => relation.replace(/^legacy-type:/, 'Legacy ').replace(/^legacy-status:/, 'Stored status: ').replace(/^legacy:/, 'Legacy: ').replaceAll('_', ' ');
const isRecordSource = (source: SourceMessage) => source.role === 'record' || source.role === 'audit';
const sourceLabel = (source: SourceMessage) => `${isRecordSource(source) ? 'Record' : 'Message'} ${source.order}`;
const dateLabel = (value: string | null) => value ? new Date(value).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' }) : 'Not established';
const shortDate = (value: string | null) => value ? new Date(value).toLocaleDateString(undefined, { month: 'short', day: 'numeric' }) : 'Undated';
function download(name: string, payload: unknown) {
  const url = URL.createObjectURL(new Blob([JSON.stringify(payload, null, 2)], { type: 'application/json' }));
  const anchor = document.createElement('a'); anchor.href = url; anchor.download = name; anchor.click();
  window.setTimeout(() => URL.revokeObjectURL(url), 2000);
}
function Badge({ children, tone = '' }: { children: ReactNode; tone?: string }) { return <span className={`badge ${tone}`}>{children}</span>; }
function SourceText({ message, spans }: { message: SourceMessage; spans: Span[] }) {
  const container = useRef<HTMLDivElement>(null);
  const ranges = spans.filter(s => s.messageId === message.id);
  const rangeKey = ranges.map(range => `${range.start}:${range.end}`).join(',');
  useEffect(() => {
    const element = container.current, mark = element?.querySelector('mark');
    if (element && mark) element.scrollTop += mark.getBoundingClientRect().top - element.getBoundingClientRect().top - 45;
  }, [message.id, rangeKey]);
  const boundaries = [...new Set([0, message.content.length, ...ranges.flatMap(s => [s.start, s.end])])].sort((a, b) => a - b);
  return <div className="source-text" data-testid="source-content" ref={container}>{boundaries.slice(0, -1).map((start, i) => {
    const end = boundaries[i + 1], text = message.content.slice(start, end);
    return ranges.some(s => start >= s.start && end <= s.end) ? <mark key={start} data-testid="source-highlight">{text}</mark> : <span key={start}>{text}</span>;
  })}</div>;
}
function ProvenanceDetails({ provenance, bundle }: { provenance: Provenance; bundle: ReasoningBundle }) {
  const actor = (id: string | null) => bundle.actors.find(a => a.id === id)?.name ?? 'Not established';
  return <dl className="provenance-grid" data-testid="reported-provenance">
    <div><dt>Expressed by</dt><dd>{actor(provenance.expressedBy)}</dd></div>
    <div><dt>Reported recorder</dt><dd>{actor(provenance.recordedBy)}</dd></div>
    <div><dt>Occurred</dt><dd>{dateLabel(provenance.occurredAt)}</dd></div>
    <div><dt>Reported recording time</dt><dd>{dateLabel(provenance.recordedAt)}</dd></div>
  </dl>;
}
function CaptureDetails({ capture, hosted, sessionActor }: { capture?: CaptureMetadata[string]; hosted: boolean; sessionActor: WorkspaceActor | null }) {
  if (!hosted) return <p className="interpretation-note" data-testid="imported-attribution">Attribution comes from this bundle and has not been verified by Loomtracer. Opening the file does not upload it.</p>;
  if (!capture) return <p className="muted">Authenticated capture metadata is not established for this record.</p>;
  // Portable actor names are source claims. Only the server session can name an
  // authenticated actor; otherwise show the exact authenticated identifier.
  const name = sessionActor?.id === capture.actorId ? `${sessionActor.name} (${capture.actorId})` : capture.actorId;
  return <><dl className="provenance-grid" data-testid="server-capture"><div><dt>Captured by</dt><dd>{name}</dd></div><div><dt>Captured at</dt><dd><time dateTime={capture.recordedAt}>{dateLabel(capture.recordedAt)}</time></dd></div></dl><p className="muted">Verified by hosted storage. Capturing a record does not establish its authorship or endorse its content.</p></>;
}
function RecordHeading({ selected, entry, connection, source }: { selected: Selection; entry?: Entry; connection?: Connection; source?: SourceMessage }) {
  return <div className="inspector-heading"><div><p className="eyebrow">{selected.kind === 'source' ? source && isRecordSource(source) ? 'Preserved source record' : 'Original message' : selected.kind === 'connection' ? 'Connection' : 'Contribution'}</p><h2>{connection ? relationLabel(connection.relation) : source ? sourceLabel(source) : entry?.labels[0] ? relationLabel(entry.labels[0]) : 'Entry'}</h2></div><code>{selected.id}</code></div>;
}

export default function ReasoningWorkspace({ mode = 'local', initialProject = '' }: { mode?: 'local' | 'hosted'; initialProject?: string }) {
  const [hosted, setHosted] = useState<HostedState | null>(null);
  const [sessionActor, setSessionActor] = useState<WorkspaceActor | null>(null);
  const [bundle, setBundle] = useState<ReasoningBundle | null>(null);
  const [runId, setRunId] = useState('');
  const [cutoff, setCutoff] = useState(0);
  const [selected, setSelected] = useState<Selection | null>(null);
  const [actorFilter, setActorFilter] = useState('');
  const [roleFilter, setRoleFilter] = useState('');
  const [tab, setTab] = useState<'timeline' | 'graph' | 'sources'>('timeline');
  const [graphOpened, setGraphOpened] = useState(false);
  const [warnings, setWarnings] = useState<string[]>([]);
  const [errors, setErrors] = useState<string[]>([]);
  const [dirty, setDirty] = useState(false);
  const [notice, setNotice] = useState('');
  const [reviewer, setReviewer] = useState('');
  const [reviewerKind, setReviewerKind] = useState<'human' | 'agent' | 'unknown'>('human');
  const [reviewComment, setReviewComment] = useState('');
  const [reviewTarget, setReviewTarget] = useState<'selected' | 'missing'>('selected');
  const [reviewOpen, setReviewOpen] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const inspectorRef = useRef<HTMLDivElement>(null);
  const view = useMemo(() => bundle?.runs.length ? getView(bundle, runId || undefined, cutoff) : null, [bundle, runId, cutoff]);
  // Anchor positions to the complete selected revision. Moving the cutoff only
  // reveals records; it never recalculates the positions of surviving records.
  const fullView = useMemo(() => bundle?.runs.length ? getView(bundle, runId || undefined) : null, [bundle, runId]);
  const entryRoles = useMemo(() => {
    const roles = new Map(view?.entries.map(entry => [entry.id, new Set(entry.labels)]));
    view?.connections.forEach(connection => {
      connection.participants.forEach(participant => roles.get(participant.entryId)?.add(participant.role));
      if (connection.rationaleEntryId) roles.get(connection.rationaleEntryId)?.add('rationale');
    });
    return roles;
  }, [view]);
  const visibleEntries = useMemo(() => view?.entries.filter(entry => (!actorFilter || entry.provenance.expressedBy === actorFilter) && (!roleFilter || entryRoles.get(entry.id)?.has(roleFilter))) ?? [], [view, actorFilter, roleFilter, entryRoles]);
  const graphState = useMemo(() => {
    if (!view) return null;
    const matching = new Set(visibleEntries.map(entry => entry.id));
    const participants = (connection: Connection) => [...connection.participants.map(p => p.entryId), ...(connection.rationaleEntryId ? [connection.rationaleEntryId] : [])];
    const linked = new Set(view.connections.flatMap(participants));
    // Keep complete reasoning steps around filter matches. Filtering an actor
    // must not cut their answers away from another actor's question or reasons.
    const connections = actorFilter || roleFilter ? view.connections.filter(connection => participants(connection).some(id => matching.has(id))) : view.connections;
    const contextIds = new Set(connections.flatMap(participants));
    const unlinked = visibleEntries.filter(entry => !linked.has(entry.id));
    const entries = view.entries.filter(entry => contextIds.has(entry.id) || matching.has(entry.id));
    return { view: { ...view, entries, connections }, unlinked, contextCount: entries.filter(entry => !matching.has(entry.id)).length };
  }, [view, visibleEntries, actorFilter, roleFilter]);
  const selectedEntry = view?.entries.find(e => selected?.kind === 'entry' && e.id === selected.id);
  const selectedConnection = [...(view?.connections ?? []), ...(view?.externalConnections ?? [])].find(c => selected?.kind === 'connection' && c.id === selected.id);
  const selectedSource = bundle?.sources.find(s => selected?.kind === 'source' && s.id === selected.id && s.order <= cutoff);
  const selectedRecord = selectedEntry ?? selectedConnection;
  const hasRecordSources = bundle?.sources.some(isRecordSource) ?? false;
  const sourceUnit = hasRecordSources ? 'record' : 'message';
  const selectedIsLegacy = bundle?.contexts.find(context => context.id === selectedRecord?.contextId)?.kind === 'legacy_record';
  const spans = selectedRecord?.provenance.sourceSpans ?? [];
  const sourceMessages = selectedSource ? [selectedSource] : (bundle?.sources.filter(s => spans.some(span => span.messageId === s.id)) ?? []);
  const activeRun = view?.run;
  const labels = [...new Set([...entryRoles.values()].flatMap(roles => [...roles]))].sort();
  const actorName = (id: string | null | undefined) => bundle?.actors.find(a => a.id === id)?.name ?? 'Not established';
  const sourceOrder = (entry: Entry) => Math.min(Infinity, ...entry.provenance.sourceSpans.map(span => bundle?.sources.find(s => s.id === span.messageId)?.order ?? Infinity));
  const timelineEntries = [...visibleEntries].sort((a, b) => sourceOrder(a) - sourceOrder(b) || Math.min(Infinity, ...a.provenance.sourceSpans.map(span => span.start)) - Math.min(Infinity, ...b.provenance.sourceSpans.map(span => span.start)) || (a.provenance.occurredAt ?? '').localeCompare(b.provenance.occurredAt ?? '') || a.id.localeCompare(b.id));
  const previous = new Set<string>();
  const lineage: NonNullable<typeof activeRun>[] = [];
  let ancestor = activeRun;
  while (ancestor && !previous.has(ancestor.id)) { previous.add(ancestor.id); lineage.unshift(ancestor); ancestor = bundle?.runs.find(r => r.id === ancestor?.parentId); }
  // Run notes and review resolutions are unanchored summaries of that run's
  // complete coverage. Never back-project them into an earlier source cutoff.
  const eligibleRuns = lineage.filter(run => run.sourceCutoff <= cutoff);
  const eligibleRunIds = new Set(eligibleRuns.map(run => run.id));
  const metadataRun = eligibleRuns.at(-1);
  const fullBundleCutoff = Math.max(0, ...(bundle?.sources.map(source => source.order) ?? []));
  const completeBundleView = cutoff >= fullBundleCutoff && (!activeRun || activeRun.id === bundle?.runs.at(-1)?.id);
  const visibleWarnings = completeBundleView ? warnings : [];
  const notes = bundle?.reviewNotes.filter(note => eligibleRunIds.has(note.runId)) ?? [];
  const resolution = (id: string) => [...eligibleRuns].reverse().flatMap(run => run.reviewResolutions).find(item => item.noteId === id);
  const unresolvedCount = notes.filter(n => resolution(n.id)?.status !== 'addressed').length;
  const activeSourceCount = bundle?.sources.filter(s => s.active && s.order <= cutoff).length ?? 0;

  useEffect(() => {
    const handler = (event: BeforeUnloadEvent) => { if (dirty) { event.preventDefault(); event.returnValue = ''; } };
    const navigation = (event: MouseEvent) => {
      if (!dirty || event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey) return;
      const link = event.target instanceof Element ? event.target.closest('a[href]') : null;
      if (!(link instanceof HTMLAnchorElement) || link.download || link.target === '_blank' || link.hash && link.pathname === location.pathname) return;
      if (!window.confirm('Your changes have not been saved or exported. Leave this history and discard those changes?')) { event.preventDefault(); event.stopImmediatePropagation(); }
    };
    window.addEventListener('beforeunload', handler); document.addEventListener('click', navigation, true);
    return () => { window.removeEventListener('beforeunload', handler); document.removeEventListener('click', navigation, true); };
  }, [dirty]);
  useEffect(() => { inspectorRef.current?.scrollTo({ top: 0 }); }, [selected?.id]);

  function loadBundle(input: unknown, label: string, preserveView = false) {
    const result = validateBundle(input);
    if (!result.ok || !result.bundle) { setErrors(result.errors); return false; }
    if (!preserveView && dirty && !window.confirm('Your review notes have not been exported in a bundle. Replace this session and discard those unsaved notes?')) return false;
    const loaded = result.bundle, latest = loaded.runs.at(-1);
    setBundle(loaded); setRunId(latest?.id ?? ''); setCutoff(latest?.sourceCutoff ?? Math.max(0, ...loaded.sources.map(source => source.order))); if (!preserveView) { setSelected(null); setReviewComment(''); setReviewOpen(false); } setWarnings(result.warnings); setErrors([]); setDirty(false); if (!preserveView) { setActorFilter(''); setRoleFilter(''); setTab(latest ? 'timeline' : 'sources'); setGraphOpened(false); } setNotice(label); return true;
  }
  async function importFile(event: ChangeEvent<HTMLInputElement>) {
    const file = event.target.files?.[0]; event.target.value = ''; if (!file) return;
    try {
      if (file.size > 60 * 1024 * 1024) throw new Error('Loomtracer accepts bundles up to 60 MB.');
      if (loadBundle(JSON.parse(await file.text()), `Imported ${file.name}. Everything stays in this browser session.`)) setHosted(null);
    } catch (error) { setErrors([error instanceof Error ? error.message : 'This file could not be read. Choose a valid JSON bundle.']); }
  }
  function exportBundle() {
    if (!bundle) return; download(`${bundle.id.replace(/[^a-zA-Z0-9_-]/g, '-')}.json`, bundle); setDirty(false); setNotice('Bundle exported with source material and review notes.');
  }
  function addReview() {
    if (!bundle || !activeRun || !reviewComment.trim() || (!reviewer.trim() && !(hosted && sessionActor)) || (!selected && reviewTarget === 'selected')) return;
    const reviewerActor = hosted && sessionActor ? sessionActor : bundle.actors.find(actor => actor.id.startsWith('local-reviewer:') && actor.name === reviewer.trim() && actor.kind === reviewerKind) ?? { id: `local-reviewer:${crypto.randomUUID()}`, name: reviewer.trim(), kind: reviewerKind };
    const note: ReviewNote = { id: `review:${crypto.randomUUID()}`, reviewer: reviewerActor.id, createdAt: new Date().toISOString(), runId: activeRun.id, target: reviewTarget === 'missing' ? { kind: 'missing', id: null } : { kind: selected!.kind, id: selected!.id }, sourceSpans: spans, comment: reviewComment.trim() };
    setBundle({ ...bundle, actors: bundle.actors.some(actor => actor.id === reviewerActor.id) ? bundle.actors : [...bundle.actors, reviewerActor], reviewNotes: [...bundle.reviewNotes, note] }); setDirty(true); setReviewComment(''); setReviewOpen(false); setNotice(cutoff < activeRun.sourceCutoff ? 'Review note added to the selected extraction revision. It is withheld until that revision’s full conversation cutoff. Export the bundle to keep it.' : 'Review note added. Export the bundle to keep it.');
  }
  function select(next: Selection) { setSelected(next); setReviewComment(''); setReviewOpen(false); }
  function inspectEntry(id: string) {
    if (!bundle) return;
    if (!view?.entries.some(entry => entry.id === id)) {
      const revision = [...bundle.runs].reverse().find(run => getView(bundle, run.id).entries.some(entry => entry.id === id));
      if (!revision) { setNotice('This contribution is not included in an extraction view. Its original record remains in the exported bundle.'); return; }
      setRunId(revision.id); setCutoff(revision.sourceCutoff);
    }
    select({ kind: 'entry', id });
  }

  return <div className="reasoning-surface"><div className="reasoning-app">
    {mode === 'local' && <input ref={inputRef} data-testid="bundle-input" aria-label="Import reasoning bundle" type="file" accept=".json,application/json" onChange={importFile} className="file-input" />}
    {mode === 'local' && <header className="masthead">
      <div className="brand-lockup"><img src="/reasoning/brand/logo.svg" width="86" height="68" alt="Loomtracer" /><div className="brand-separator" /><div><span className="product-name">Inspect a bundle</span><span className="product-meta">{hosted ? 'Connected to your project' : 'Private file inspection · browser memory only'}</span></div></div>
      <div className="header-actions">{dirty && <span className="unsaved" role="status" data-testid="review-unsaved"><span />Unsaved changes</span>}<button className="button secondary" onClick={() => inputRef.current?.click()}><span aria-hidden="true">↥</span> Import bundle</button>{bundle && <button className="button primary" onClick={exportBundle} data-testid="export-bundle"><span aria-hidden="true">↧</span> Export bundle</button>}</div>
    </header>}
    <HostedControls mode={mode} initialProject={initialProject} bundle={bundle} dirty={dirty} historical={!completeBundleView} selectedEntry={selectedEntry} hosted={hosted} setHosted={setHosted} onActor={setSessionActor} onLoad={(next, label, preserveView) => loadBundle(next, label, preserveView)} onInspectEntry={inspectEntry} onError={message => setErrors([message])} onNotice={setNotice} />
    {errors.length > 0 && <div className="error-banner" role="alert"><strong>The operation could not be completed.</strong><ul>{errors.slice(0, 8).map((error, index) => <li key={index}>{error}</li>)}</ul><button className="text-button" onClick={() => setErrors([])}>Dismiss</button></div>}
    {!bundle ? mode === 'local' && <main className="welcome">
      <div className="welcome-copy"><p className="eyebrow"><span className="eyebrow-line" /> From conversation to continuity</p><h1>See how a decision<br />came to be.</h1><p className="welcome-intro">Follow the contributions, reasons, and changes of mind that shaped a direction. Then check every connection against its source.</p><div className="welcome-actions"><button className="button primary" onClick={() => inputRef.current?.click()}>Import your conversation <span aria-hidden="true">↗</span></button><button className="text-button" onClick={() => loadBundle(makeDemoBundle(), 'Synthetic walkthrough loaded. These are illustrative records, not your conversation.')}>Explore a synthetic example <span aria-hidden="true">→</span></button></div><p className="privacy-note"><span aria-hidden="true">○</span> Files stay in this browser’s memory. Export before leaving.</p></div>
      <div className="welcome-illustration" aria-label="An illustration of a proposal, adoption, and later challenge"><div className="illustration-topline"><span>ONE THREAD. DIFFERENT MOMENTS.</span><span>01—03</span></div><div className="illustration-step"><span className="step-number blue">01</span><div><p className="eyebrow">Monday · an agent</p><h2>A proposal takes shape.</h2><p>“A meets the requirement at a lower cost.”</p><span className="mini-label">Recommendation</span></div></div><div className="illustration-link"><span />adopts</div><div className="illustration-step"><span className="step-number yellow">02</span><div><p className="eyebrow">Tuesday · a human</p><h2>A direction is chosen.</h2><p>“Let’s proceed with A.”</p><span className="mini-label">Commitment</span></div></div><div className="illustration-link"><span />challenges the earlier premise</div><div className="illustration-step"><span className="step-number orange">03</span><div><p className="eyebrow">Friday · another agent</p><h2>New evidence changes the picture.</h2><p>“The new load test exceeds A’s capacity.”</p><span className="mini-label">Challenge ≠ withdrawal</span></div></div><div className="illustration-foot">Illustrative sequence <span>Preserve the change. Keep the source.</span></div></div>
      <div className="welcome-principles"><div><span>01 / CONTRIBUTIONS</span><p>Who said what,<br />and when.</p></div><div><span>02 / CONNECTIONS</span><p>What supported,<br />challenged, or changed it.</p></div><div><span>03 / PROVENANCE</span><p>The source survives<br />every interpretation.</p></div></div>
    </main> : <>
      <section className="workspace-heading"><div><p className="eyebrow">Reasoning history</p><h1>{bundle.title}</h1><p className="dataset-subtitle">{activeSourceCount} source {sourceUnit}s <span>·</span> {view?.entries.length ?? 0} contributions <span>·</span> {view?.connections.length ?? 0} connections{Boolean(view?.externalConnections?.length) && <> <span>·</span> {view?.externalConnections?.length} cross-project links</>}</p></div><div className="session-caption"><span className="session-dot" />{hosted ? 'Saved project history' : 'Browser memory only'}<span className="session-caption-detail">{dirty ? (hosted ? 'Save changes or export to preserve them.' : 'Export to preserve your review.') : (hosted ? 'Changes create new history; originals remain.' : 'No data is sent to a server.')}</span></div></section>
      <div className="workspace-controls"><div className="revision-control"><label htmlFor="revision-select">Extraction revision</label><select id="revision-select" data-testid="run-select" value={runId} onChange={event => { const run = bundle.runs.find(r => r.id === event.target.value)!; setRunId(run.id); setCutoff(run.sourceCutoff); }}><option value="" disabled>{bundle.runs.length ? 'Select revision' : 'Source only · no extraction'}</option>{bundle.runs.map((run, i) => <option key={run.id} value={run.id}>{i + 1}. {run.id}</option>)}</select></div><div className="cutoff-control"><div className="cutoff-label"><label htmlFor="cutoff-range">{hasRecordSources ? 'Sources through' : 'Conversation through'}</label><span>{hasRecordSources ? 'Record' : 'Message'} {cutoff}<span className="muted"> / {activeRun?.sourceCutoff ?? Math.max(0, ...bundle.sources.map(s => s.order))}</span></span></div><input id="cutoff-range" data-testid="cutoff-slider" type="range" min={Math.min(...bundle.sources.filter(s => s.active).map(s => s.order), 0)} max={activeRun?.sourceCutoff ?? Math.max(0, ...bundle.sources.map(s => s.order))} value={cutoff} onChange={event => setCutoff(Number(event.target.value))} /></div><div className="review-count"><span className="review-count-number">{unresolvedCount.toString().padStart(2, '0')}</span><span>open review<br />{unresolvedCount === 1 ? 'note' : 'notes'}</span></div></div>
      <details className="extraction-details">
        <summary><span>Extraction & source coverage</span><span>{metadataRun ? `${metadataRun.uncertainties.length} uncertainties` : activeRun ? 'Notes withheld at this cutoff' : 'Source only'} <span aria-hidden="true">⌄</span></span></summary>
        <div className="extraction-detail-body">
          <div>
            <p className="eyebrow">Selected extraction revision</p>
            {activeRun ? <><p className="muted">Recorded {dateLabel(activeRun.createdAt)} by {actorName(activeRun.recordedBy)}. This interpretation is retrospective; source order determines the historical view.</p><p>Selected revision totals: {activeRun.addedEntryIds.length} entries and {activeRun.addedConnectionIds.length} connections added; {activeRun.replacements.length} replacements and {activeRun.omissions?.length ?? 0} records returned to source-only. Coverage extends through message {activeRun.sourceCutoff}.</p></> : <p>Source-only bundle. The original messages can be inspected; reasoning entries and connections have not been extracted.</p>}
            {metadataRun ? <><p className="eyebrow">Notes through message {metadataRun.sourceCutoff} · {metadataRun.id}</p><p>{metadataRun.notes || 'No extraction notes were provided for this revision.'}</p>{metadataRun.replacements.map(replacement => <p className="replacement" key={replacement.previousId}><code>{replacement.previousId}</code> → <code>{replacement.nextId}</code></p>)}{(metadataRun.omissions?.length ?? 0) > 0 && <details className="omission-details"><summary>Why {metadataRun.omissions!.length} records are source-only in this revision</summary><ul>{metadataRun.omissions!.map(item => <li key={`${item.kind}:${item.recordId}`}><code>{item.recordId}</code><p>{item.reason}</p></li>)}</ul><p className="muted">Earlier extraction revisions and the original messages are preserved. This corrects the extraction; it does not withdraw anyone’s decision.</p></details>}</> : activeRun ? <p className="interpretation-note">The selected extraction is retrospective and its notes cover later messages. Notes and uncertainties are withheld at this cutoff because no ancestor revision ends within it.</p> : null}
          </div>
          <div>
            <p className="eyebrow">Limits & uncertainties at this cutoff</p>
            {[...(metadataRun?.uncertainties ?? []), ...visibleWarnings].length > 0 && <ul>{[...(metadataRun?.uncertainties ?? []), ...visibleWarnings].map((warning, index) => <li key={index}>{warning}</li>)}</ul>}
            {metadataRun && !metadataRun.uncertainties.length && !visibleWarnings.length && <p>No uncertainty notes were provided for this snapshot. This is not a correctness guarantee.</p>}
            {!completeBundleView && warnings.length > 0 && <p className="muted">{warnings.length} bundle validation warnings are withheld here; they may refer to records beyond this cutoff. Inspect the latest revision at its full cutoff to read them.</p>}
            <p className="muted">Source frozen {dateLabel(bundle.manifest.frozenAt)}.{completeBundleView ? ` ${bundle.manifest.cutoffDescription}` : ' Freeze description is withheld in this historical view.'}</p>
            {activeRun && <p className="muted">Selected revision coverage: {activeRun.sourceMessageIds.length} source {sourceUnit}s.</p>}
          </div>
        </div>
      </details>
      <main className={`workspace ${tab === 'graph' ? 'workspace-graph' : ''}`}><section className="trace-panel" aria-label="Reasoning history"><div className="panel-navigation"><div className="tabs" role="tablist" aria-label="History view"><button role="tab" data-testid="timeline-tab" aria-selected={tab === 'timeline'} onClick={() => setTab('timeline')}>Timeline</button><button role="tab" data-testid="graph-tab" aria-selected={tab === 'graph'} onClick={() => { setGraphOpened(true); setTab('graph'); }}>Trace map</button><button role="tab" aria-selected={tab === 'sources'} onClick={() => setTab('sources')}>Sources</button></div><span className="panel-note">{tab === 'sources' ? hasRecordSources ? 'Preserved sources' : 'Original conversation' : 'Follow the reasoning'}</span></div>
        {tab !== 'sources' && <div className="filters"><label><span>Actor</span><select aria-label="Filter by actor" data-testid="actor-filter" value={actorFilter} onChange={event => setActorFilter(event.target.value)}><option value="">Everyone</option>{bundle.actors.filter(a => view?.entries.some(e => e.provenance.expressedBy === a.id)).map(a => <option key={a.id} value={a.id}>{a.name}</option>)}</select></label><label><span>Role</span><select aria-label="Filter by contribution role" data-testid="role-filter" value={roleFilter} onChange={event => setRoleFilter(event.target.value)}><option value="">All roles</option>{labels.map(label => <option key={label} value={label}>{relationLabel(label)}</option>)}</select></label>{(actorFilter || roleFilter) && <button className="text-button reset-filters" onClick={() => { setActorFilter(''); setRoleFilter(''); }}>Reset</button>}<span className="result-count">{visibleEntries.length} entries</span></div>}
        {tab === 'timeline' && <div className="timeline" data-testid="timeline">{timelineEntries.length ? timelineEntries.map((entry, index) => {
          const context = bundle.contexts.find(c => c.id === entry.contextId);
          const connections = relatedConnections(view!, entry.id);
          return <article key={entry.id} className={`timeline-item ${selected?.id === entry.id ? 'is-selected' : ''}`} data-testid="timeline-entry"><div className="timeline-rail"><span className={`timeline-dot ${bundle.actors.find(a => a.id === entry.provenance.expressedBy)?.kind === 'human' ? 'human' : 'agent'}`} />{index < timelineEntries.length - 1 && <span className="timeline-line" />}</div><div className="timeline-content"><button className="entry-button" data-testid={`entry-card-${entry.id}`} onClick={() => select({ kind: 'entry', id: entry.id })}><div className="entry-topline"><span className="entry-actor">{actorName(entry.provenance.expressedBy)}</span><span>{Number.isFinite(sourceOrder(entry)) ? `${hasRecordSources ? 'S' : 'M'}${sourceOrder(entry)} · ` : ''}{shortDate(entry.provenance.occurredAt)}</span></div><div className="entry-badges">{entry.labels.map(label => <Badge key={label}>{relationLabel(label)}</Badge>)}{entry.provenance.basis === 'inferred' && <Badge tone="inferred">Inferred</Badge>}{context?.kind !== 'discussion' && <Badge tone="illustration">{context?.kind === 'illustration' ? 'Illustration' : context?.kind === 'legacy_record' ? 'Legacy record' : 'Quoted source'}</Badge>}</div><p className="entry-quote">{entry.summary ?? entryText(bundle, entry)}</p><div className="entry-bottom"><code>{entry.id}</code><span>{entry.summary ? 'Generated summary · inspect source' : 'Exact source passage'} <span aria-hidden="true">↗</span></span></div></button>{connections.length > 0 && <div className="entry-connections">{connections.map(connection => <button key={connection.id} className={`connection-chip ${selected?.id === connection.id ? 'is-selected' : ''}`} onClick={() => select({ kind: 'connection', id: connection.id })} title={`Inspect ${connection.id}`}><span aria-hidden="true">↳</span> {relationLabel(connection.relation)} <span className="connection-chip-id">{connection.id}</span>{connection.provenance.basis === 'inferred' && <span className="inferred-dot" aria-label="Inferred connection" />}</button>)}</div>}</div></article>;
        }) : <div className="empty-state"><h2>No contributions in this view.</h2><p>Adjust the actor, role, revision, or conversation cutoff.</p></div>}</div>}
        {graphOpened && graphState && fullView && <div hidden={tab !== 'graph'} className="graph-pane">
          <div className="graph-context-bar">
            <span>{graphState.view.entries.length} entries{graphState.unlinked.length > 0 ? ` · ${graphState.unlinked.length} not yet connected` : ''}{graphState.contextCount > 0 ? ` · ${graphState.contextCount} context entries retained` : ''}</span>
          </div>
          {graphState.unlinked.length > 0 && <details className="unlinked-details"><summary>{graphState.unlinked.length} contributions have no connections at this source cutoff</summary><p>These contributions remain preserved, with no captured connection at this source cutoff. A missing connection is not evidence of a relationship.</p><div>{graphState.unlinked.map(entry => <button key={entry.id} className="text-button" onClick={() => select({ kind: 'entry', id: entry.id })}>{entryText(bundle, entry).slice(0, 95)}<code>{entry.id}</code></button>)}</div></details>}
          {Boolean(view?.externalConnections?.length) && <details className="unlinked-details"><summary>{view?.externalConnections?.length} preserved cross-project links</summary><p>Other projects’ contribution text is not included here. Inspect the preserved relationship and source without copying those entries into this project.</p><div>{view?.externalConnections?.map(connection => <button key={connection.id} className="text-button" onClick={() => select({ kind: 'connection', id: connection.id })}>{relationLabel(connection.relation)} <code>{connection.id}</code></button>)}</div></details>}
          <Suspense fallback={<div className="empty-state" role="status">Preparing the trace map…</div>}><GraphView bundle={bundle} view={graphState.view} fullView={fullView} selected={selected} onSelect={select} /></Suspense>
        </div>}
        {tab === 'sources' && <div className="source-list">{bundle.sources.filter(s => s.order <= cutoff).sort((a, b) => a.order - b.order).map(source => <button key={source.id} className={`source-row ${selected?.id === source.id ? 'is-selected' : ''}`} onClick={() => select({ kind: 'source', id: source.id })}><span className="source-order">{String(source.order).padStart(2, '0')}</span><div><div className="entry-topline"><strong>{actorName(source.actorId)}</strong><span>{shortDate(source.occurredAt)}</span></div><p>{source.content.slice(0, 190)}{source.content.length > 190 ? '…' : ''}</p><span className="source-row-foot">{source.active ? isRecordSource(source) ? 'Preserved source record' : 'Active conversation' : 'Alternative source occurrence'} · {source.id}</span></div><span aria-hidden="true">↗</span></button>)}</div>}
      </section>
      <aside className="inspector" data-testid="inspector" aria-label="Source inspector" ref={inspectorRef}>{selected && (selectedEntry || selectedConnection || selectedSource) ? <>
        <RecordHeading selected={selected} entry={selectedEntry} connection={selectedConnection} source={selectedSource} />
        {selectedRecord && <><div className="inspector-badges"><Badge tone={selectedRecord.provenance.basis === 'inferred' ? 'inferred' : 'direct'}>{selectedRecord.provenance.basis === 'inferred' ? 'Later inference' : selectedIsLegacy ? 'Preserved source' : 'Directly expressed'}</Badge><Badge>{bundle.contexts.find(c => c.id === selectedRecord.contextId)?.kind === 'illustration' ? 'Illustration' : bundle.contexts.find(c => c.id === selectedRecord.contextId)?.kind === 'quoted_source' ? 'Quoted material' : bundle.contexts.find(c => c.id === selectedRecord.contextId)?.kind === 'legacy_record' ? 'Legacy record' : 'Discussion'}</Badge></div>{selectedEntry && <div className="selected-content"><p>{entryText(bundle, selectedEntry)}</p>{selectedEntry.summary && <details><summary>Generated summary</summary><p>{selectedEntry.summary}</p></details>}</div>}
        {selectedConnection && <div className="participants"><p className="eyebrow">Participants</p>{selectedConnection.participants.map((participant, index) => { const entry = view?.entries.find(e => e.id === participant.entryId); const external = bundle.externalEntryRefs.find(ref => ref.id === participant.entryId); return <button disabled={!entry} key={`${participant.entryId}:${index}`} onClick={() => select({ kind: 'entry', id: participant.entryId })}><span className="participant-role">{relationLabel(participant.role)}</span><span>{entry ? (entry.summary ?? entryText(bundle, entry)).slice(0, 180) : external ? `External contribution ${external.originalId} · project ${external.namespace.projectId ?? 'not established'}` : participant.entryId}<code>{participant.entryId} ↗</code></span></button>; })}{selectedConnection.rationaleEntryId && <button className="text-button" onClick={() => select({ kind: 'entry', id: selectedConnection.rationaleEntryId! })}>Read linked rationale ↗</button>}</div>}
        <ProvenanceDetails provenance={selectedRecord.provenance} bundle={bundle} />{selectedRecord.provenance.basis === 'inferred' && <p className="interpretation-note">This connection or contribution is an extraction interpretation. Its existence does not establish the original actor’s unspoken reasoning.</p>}</>}
        {selectedSource && <dl className="provenance-grid"><div><dt>{isRecordSource(selectedSource) ? 'Source recorder' : 'Speaker'}</dt><dd>{actorName(selectedSource.actorId)}</dd></div><div><dt>Occurred</dt><dd>{dateLabel(selectedSource.occurredAt)}</dd></div><div><dt>Logged</dt><dd>{dateLabel(selectedSource.loggedAt)}</dd></div><div><dt>Sequence</dt><dd>{selectedSource.order}{!selectedSource.active && ' · alternate occurrence'}</dd></div><div><dt>Reported recorder</dt><dd>{actorName(selectedSource.provenance?.recordedBy)}</dd></div><div><dt>Reported recording time</dt><dd>{dateLabel(selectedSource.provenance?.recordedAt ?? null)}</dd></div></dl>}
        <CaptureDetails capture={hosted?.captureMetadata[selected.id]} hosted={Boolean(hosted)} sessionActor={sessionActor} />
        <div className="source-section"><div className="section-label"><h3>At the source</h3><span>{sourceMessages.length} {sourceMessages.some(isRecordSource) ? sourceMessages.length === 1 ? 'record' : 'records' : sourceMessages.length === 1 ? 'message' : 'messages'}</span></div>{sourceMessages.length ? sourceMessages.map(message => <section className="source-passage" data-testid="source-passage" key={message.id}><div className="source-passage-header"><strong>{isRecordSource(message) ? `Preserved ${message.provenance?.originalRecord?.table ?? message.role} record` : actorName(message.actorId)}</strong><button className="text-button" onClick={() => select({ kind: 'source', id: message.id })}>{sourceLabel(message)} ↗</button></div><SourceText message={message} spans={spans} />{message.attachmentIds.map(id => { const attachment = bundle.attachments.find(a => a.id === id); return attachment && /^data:image\/(png|jpeg|webp);base64,[A-Za-z0-9+/=\r\n]+$/.test(attachment.dataUrl) ? <figure className="source-attachment" key={id}><img src={attachment.dataUrl} alt={`Source attachment ${id}`} /><figcaption>{id}</figcaption></figure> : null; })}<details className="source-origin"><summary>Source identity & timestamps</summary><dl><dt>Original ID</dt><dd>{message.originalId}</dd><dt>Source reference</dt><dd>{message.sourceRef}</dd><dt>{isRecordSource(message) ? 'Occurrence time' : 'Message time'}</dt><dd>{dateLabel(message.occurredAt)}</dd><dt>Log time</dt><dd>{dateLabel(message.loggedAt)}</dd></dl></details></section>) : <p className="muted">No original source passage was provided. This entry may be a newly authored contribution.</p>}</div>
        <div className="review-section"><div className="section-label"><h3>Review this record</h3><button className="text-button" disabled={!activeRun} onClick={() => setReviewOpen(v => !v)}>{reviewOpen ? 'Close' : '+ Add note'}</button></div><p className="muted review-explanation">{activeRun ? 'Flag an attribution, missing reason, or extraction error. Review notes do not adopt decisions.' : 'Import an extraction revision to attach review notes to this source.'}</p>{reviewOpen && <form className="review-form" onSubmit={event => { event.preventDefault(); addReview(); }}><label>Reviewer<input disabled={Boolean(hosted && sessionActor)} aria-label="Reviewer name" value={hosted && sessionActor ? sessionActor.name : reviewer} onChange={event => setReviewer(event.target.value)} required /></label><label>Reviewer type<select disabled={Boolean(hosted && sessionActor)} aria-label="Reviewer type" value={hosted && sessionActor ? 'human' : reviewerKind} onChange={event => setReviewerKind(event.target.value as 'human' | 'agent' | 'unknown')}><option value="human">Human</option><option value="agent">Agent</option><option value="unknown">Unknown</option></select></label><label>About<select aria-label="Review target" value={reviewTarget} onChange={event => setReviewTarget(event.target.value as 'selected' | 'missing')}><option value="selected">Selected {selected.kind}</option><option value="missing">A missing contribution or connection</option></select></label><label>Review note<textarea aria-label="Review note" data-testid="review-comment" placeholder="What should be corrected or clarified?" value={reviewComment} onChange={event => setReviewComment(event.target.value)} rows={4} required /></label><button className="button primary" data-testid="review-save" type="submit">Save review note</button><span className="muted">{hosted ? 'Attributed to your signed-in account. Save changes to persist this note.' : 'Reviewer names are labels, not authenticated identities. Export to preserve this note.'}{activeRun && cutoff < activeRun.sourceCutoff ? ` This note attaches to ${activeRun.id} and is withheld until message ${activeRun.sourceCutoff}.` : ''}</span></form>}{notes.filter(n => n.target.id === selected.id).map(note => <div className="review-note" key={note.id}><div><strong>{actorName(note.reviewer)}</strong><Badge>{resolution(note.id)?.status ?? 'Open'}</Badge></div><p>{note.comment}</p><small>{dateLabel(note.createdAt)} · {note.runId}</small>{resolution(note.id)?.comment && <p className="resolution-comment">{resolution(note.id)?.comment}</p>}</div>)}</div>
      </> : <div className="inspector-empty"><div className="inspector-symbol" aria-hidden="true"><span /><span /><span /></div><p className="eyebrow">The source stays in view</p><h2>Every interpretation<br />has a place to begin.</h2><p>Select a contribution or a connection to inspect its participants, ownership, and exact source passage.</p><div className="inspector-legend"><div><span className="legend-mark direct" /><p><strong>Directly expressed</strong><br />Grounded in a recorded contribution.</p></div><div><span className="legend-mark inferred" /><p><strong>Later inference</strong><br />An attributed extraction interpretation.</p></div></div></div>}</aside>
      </main>
      {notes.length > 0 && <details className="all-reviews"><summary><span>Review notebook</span><span>{notes.length} notes · {unresolvedCount} open <span aria-hidden="true">⌄</span></span></summary><div className="all-reviews-content"><button className="button secondary" onClick={() => download(`${bundle.id}-reviews.json`, { schemaVersion: bundle.schemaVersion, bundleId: bundle.id, actors: bundle.actors.filter(actor => bundle.reviewNotes.some(note => note.reviewer === actor.id)), reviewNotes: bundle.reviewNotes })}>Export review notes</button>{notes.map(note => <article className="notebook-row" key={note.id}><div><strong>{actorName(note.reviewer)}</strong><span>{note.target.kind} · {note.target.id ?? 'Missing contribution'}</span><Badge>{resolution(note.id)?.status ?? 'Open'}</Badge></div><p>{note.comment}</p>{resolution(note.id)?.comment && <p className="muted">{resolution(note.id)?.comment}</p>}</article>)}</div></details>}
      <footer className="workspace-footer"><span>Entry + Connection <span className="footer-divider">/</span> Schema {bundle.schemaVersion}</span><span>Reconstruction is an interpretation. The original record remains inspectable.</span>{mode === 'hosted' && <button className="text-button" onClick={exportBundle} data-testid="export-bundle">Export history</button>}</footer>
    </>}
    <div className="live-status" role="status" aria-live="polite">{notice}</div>
  </div></div>;
}
