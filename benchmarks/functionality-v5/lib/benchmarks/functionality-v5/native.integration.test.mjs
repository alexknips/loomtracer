import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, readFile, writeFile, access } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { inspectNativeExecutable, applyNativeMemory, createNativeTaskView, nativeLookup, verifyNativeMemory, verifyNativeTaskView } from './native.mjs';

import { buildCorpus, selectedScreenIds, bindAcquisition } from './corpus.mjs';
import { createWorld, applyAction, validAcquisitionPlan } from './world.mjs';
import { performWorkflow } from './workflow.mjs';

const binary = process.env.HIVEMIND_BENCHMARK_BINARY;
const integration = (name, fn) => test(name, { skip: !binary ? 'Set HIVEMIND_BENCHMARK_BINARY to run the actual Rust CLI; no mocked backend counts as native proof' : false, timeout: 30000 }, fn);
const sources = [
  { id: 'src-1', order: 1, actor: 'Noah', text: 'Queue wait is high and DNS is normal. Both measurements support a bounded pool trial.' },
  { id: 'src-2', order: 2, actor: 'Ada', text: 'I adopt a staging-only trial of five jobs. Production is excluded.' },
  { id: 'src-3', order: 3, actor: 'Noah', text: 'I challenge larger batches because staffing may be inadequate; I did not withdraw the trial.' },
  { id: 'src-4', order: 4, actor: 'Ada', text: 'I now replace the trial limit with eight jobs in staging, with the same staff.' },
  { id: 'src-5', order: 5, actor: 'Recorder', text: 'Correction: Ada adopted the trial; Noah supplied the joint measurements. This is a recording correction, not a reversal.' },
];
const firstOperations = () => [
  { tool: 'capture_evidence', key: 'measurements', sourceIds: ['src-1'], args: { content: 'Noah observed high queue wait together with normal DNS timing.' } },
  { tool: 'capture_decision', key: 'pilot', sourceIds: ['src-1', 'src-2'], args: {
    title: 'Bounded staging pool trial', rationale: 'High queue wait together with normal DNS supports testing a pool increase; production remains excluded.',
    topic_keys: ['staging', 'queue'], options: [{ label: 'Bounded trial' }, { label: 'Retry DNS', description: 'Rejected because DNS timing is normal.' }],
    chosen_option_label: 'Bounded trial', decided_by: 'human:Ada', grounding: [{ kind: 'evidence', evidence_id: 'measurements' }],
    binds: { level: 'something', statement: 'Only staging trials of five jobs are authorized.' }, undo_cost: { level: 'cheap' },
  } },
];
async function fixture(t) {
  const root = await mkdtemp(join(tmpdir(), 'v5-native-integration-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const nativeConfig = await inspectNativeExecutable({ executable: binary });
  const memory = await applyNativeMemory(null, { operations: firstOperations() }, { nativeConfig, checkpointRoot: join(root, 'episode-1'), revision: 'episode-1', eligibleSources: sources.slice(0, 2), sourceCutoff: 2 });
  t.diagnostic(`Actual native backend ${nativeConfig.version}; binary SHA-256 ${nativeConfig.sha256}; zero provider calls.`);
  return { root, nativeConfig, memory };
}

integration('real native capture recalls attributed reasoning and expands evidence and rejected options', async t => {
  const { root, nativeConfig, memory } = await fixture(t);
  const view = await createNativeTaskView(memory, { nativeConfig, taskRoot: join(root, 'reasoning') });
  const recall = await nativeLookup(view, { kind: 'recall', query: 'bounded staging pool trial' });
  const decision = memory.records.find(r => r.key === 'pilot');
  assert.ok(recall.recordIds.includes(decision.id));
  assert.deepEqual(recall.sourceIds.sort(), ['src-1', 'src-2']);
  const expansion = await nativeLookup(view, { kind: 'expand', id: decision.id });
  const text = JSON.stringify(expansion.payload.native);
  assert.match(text, /High queue wait together with normal DNS/);
  assert.match(text, /human:Ada/); assert.match(text, /Retry DNS/); assert.match(text, /Noah observed/);
  assert.ok(view.queries.every(q => q.payloadHash && q.recordIds.length));
  assert.equal(await verifyNativeMemory(memory), true);
});

integration('real native grounding records a correction without changing a participant decision', async t => {
  const { root, nativeConfig, memory } = await fixture(t);
  const updated = await applyNativeMemory(memory, { operations: [
    { tool: 'capture_evidence', key: 'correction', sourceIds: ['src-5'], args: { content: sources[4].text } },
    { tool: 'ground_decision', sourceIds: ['src-5'], args: { decision_id: 'pilot', grounding: [{ kind: 'evidence', evidence_id: 'correction' }] } },
  ] }, { nativeConfig, checkpointRoot: join(root, 'episode-2'), revision: 'episode-2', eligibleSources: sources, sourceCutoff: 5 });
  const oldView = await createNativeTaskView(memory, { nativeConfig, taskRoot: join(root, 'old') });
  const newView = await createNativeTaskView(updated, { nativeConfig, taskRoot: join(root, 'new') });
  const old = await nativeLookup(oldView, { kind: 'expand', id: 'pilot' });
  const current = await nativeLookup(newView, { kind: 'expand', id: 'pilot' });
  assert.doesNotMatch(JSON.stringify(old.payload), /recording correction/);
  assert.match(JSON.stringify(current.payload), /recording correction/);
  assert.match(JSON.stringify(current.payload.native), /accepted/);
  assert.ok(!JSON.stringify(current.payload.native).includes('"status":"superseded"'));
  assert.equal(await verifyNativeMemory(memory), true);
  assert.notEqual(memory.db.directory, updated.db.directory); assert.notEqual(memory.db.sha256, updated.db.sha256);
});

integration('real native challenge remains contested and actual supersession has a historical replacement link', async t => {
  const { root, nativeConfig, memory } = await fixture(t);
  const challenged = await applyNativeMemory(memory, { operations: [
    { tool: 'disagree_decision', sourceIds: ['src-3'], args: { decision_id: 'pilot', actor_id: 'human:Noah', reason: 'Larger batches may exceed available staffing; challenge does not withdraw the staging trial.' } },
  ] }, { nativeConfig, checkpointRoot: join(root, 'challenge'), revision: 'challenge', eligibleSources: sources.slice(0, 3), sourceCutoff: 3 });
  const challengedView = await createNativeTaskView(challenged, { nativeConfig, taskRoot: join(root, 'challenge-task') });
  const contested = await nativeLookup(challengedView, { kind: 'expand', id: 'pilot' });
  assert.match(JSON.stringify(contested.payload.native), /contested/);
  assert.match(JSON.stringify(contested.payload.native), /human:Noah/);
  const revised = await applyNativeMemory(challenged, { operations: [
    { tool: 'supersede_decision', key: 'revised-pilot', sourceIds: ['src-4'], args: {
      old_decision_id: 'pilot', actor_id: 'human:Ada', title: 'Eight job staging trial',
      rationale: 'Ada explicitly replaced the bounded trial limit with eight staging jobs while keeping the same staff.',
      topic_keys: ['staging', 'queue'], options: [{ label: 'Eight jobs' }], chosen_option_label: 'Eight jobs',
      grounding: [{ kind: 'decision', decision_id: 'pilot' }], binds: { level: 'something', statement: 'Staging only and the same staff.' }, undo_cost: { level: 'cheap' },
    } },
  ] }, { nativeConfig, checkpointRoot: join(root, 'revised'), revision: 'revised', eligibleSources: sources.slice(0, 4), sourceCutoff: 4 });
  const revisedView = await createNativeTaskView(revised, { nativeConfig, taskRoot: join(root, 'revised-task') });
  const revisedOld = await nativeLookup(revisedView, { kind: 'expand', id: 'pilot' });
  assert.match(JSON.stringify(revisedOld.payload.native), /supersed/);
  assert.match(JSON.stringify(revisedOld.payload.native), /Eight job staging trial/);
  assert.equal(await verifyNativeMemory(memory), true); assert.equal(await verifyNativeMemory(challenged), true);
});

integration('native source references cannot leak future evidence or another scope and failed captures preserve prior memory', async t => {
  const { root, nativeConfig, memory } = await fixture(t);
  const invalid = [
    { tool: 'capture_evidence', key: 'future', sourceIds: ['src-4'], args: { content: 'Future evidence' } },
    { tool: 'capture_evidence', key: 'foreign', sourceIds: ['external-secret'], args: { content: 'Other project evidence' } },
    { tool: 'dump_graph', sourceIds: ['src-1'], args: {} },
  ];
  for (let n = 0; n < invalid.length; n++) {
    const checkpointRoot = join(root, `rejected-${n}`);
    await assert.rejects(applyNativeMemory(memory, { operations: [invalid[n]] }, { nativeConfig, checkpointRoot, revision: `bad-${n}`, eligibleSources: sources, sourceCutoff: 2 }), /visible|Unknown/);
    await assert.rejects(access(checkpointRoot), /ENOENT/);
  }
  const scoped = firstOperations()[1]; scoped.key = 'bad-project'; scoped.args.project = 'private-account';
  await assert.rejects(applyNativeMemory(memory, { operations: [scoped] }, { nativeConfig, checkpointRoot: join(root, 'scope-rejected'), revision: 'bad-scope', eligibleSources: sources.slice(0, 2), sourceCutoff: 2 }), /scope override/);
  assert.equal(await verifyNativeMemory(memory), true);
});

integration('real native queries use independent task stores and query echoes never manufacture returned source refs', async t => {
  const { root, nativeConfig, memory: initial } = await fixture(t);
  const memory = await applyNativeMemory(initial, { operations: [{ tool: 'capture_evidence', key: 'unlinked', sourceIds: ['src-5'], args: { content: 'Unlinked independent correction evidence.' } }] }, { nativeConfig, checkpointRoot: join(root, 'unlinked'), revision: 'unlinked', eligibleSources: sources, sourceCutoff: 5 });
  const a = await createNativeTaskView(memory, { nativeConfig, taskRoot: join(root, 'task-a') });
  const b = await createNativeTaskView(memory, { nativeConfig, taskRoot: join(root, 'task-b') });
  assert.notEqual(a.directory, b.directory); assert.notEqual(a.directory, memory.db.directory);
  const evidence = memory.records.find(r => r.key === 'unlinked');
  const result = await nativeLookup(a, { kind: 'recall', query: evidence.id });
  assert.ok(!result.recordIds.includes(evidence.id), 'An unlinked evidence ID in the query echo is not returned evidence');
  assert.ok(!result.sourceIds.includes('src-5'), 'Query text alone cannot unlock the unlinked source');
  assert.equal(b.queries.length, 0);
  await assert.rejects(nativeLookup(a, { kind: 'recall', query: 'queue', project: 'foreign' }), /Unsupported/);
  await assert.rejects(nativeLookup(a, { kind: 'expand', id: 'decision-not-retained' }), /Unknown retained/);
  await assert.rejects(nativeLookup(a, { kind: 'archive', query: 'all sources' }), /Only native/);
  assert.equal(await verifyNativeMemory(memory), true);
});

integration('frozen native database tampering is rejected before task retrieval', async t => {
  const { root, nativeConfig, memory } = await fixture(t);
  const name = Object.keys(memory.db.files)[0], file = join(memory.db.directory, name);
  const saved = await readFile(file); await writeFile(file, Buffer.concat([saved, Buffer.from('tampered')]));
  await assert.rejects(verifyNativeMemory(memory), /database changed/);
  await assert.rejects(createNativeTaskView(memory, { nativeConfig, taskRoot: join(root, 'must-not-run') }), /database changed/);
});


integration('serialized native task continuation preserves exact query receipts and rejects changed clone or evidence', async t => {
  const { root, nativeConfig, memory } = await fixture(t);
  const view = await createNativeTaskView(memory, { nativeConfig, taskRoot: join(root, 'serializable') });
  const recall = await nativeLookup(view, { kind: 'recall', query: 'bounded staging' });
  const restored = JSON.parse(JSON.stringify(view));
  assert.equal(await verifyNativeTaskView(restored), true);
  assert.equal(restored.queries[0].payloadHash, recall.payloadHash);
  await nativeLookup(restored, { kind: 'expand', id: 'pilot' });
  assert.equal(restored.queries.length, 2);
  assert.equal(await verifyNativeTaskView(restored), true);
  await assert.rejects(verifyNativeTaskView(view), /database changed|receipt count changed/, 'The pre-continuation snapshot cannot replace the new task state');
  const changed = JSON.parse(JSON.stringify(restored)); changed.queries[0].payload.native.hidden = 'injected';
  await assert.rejects(verifyNativeTaskView(changed), /integrity changed/);
  const path = join(restored.directory, Object.keys(memory.db.files)[0]);
  await writeFile(path, Buffer.concat([await readFile(path), Buffer.from('unexpected-write')]));
  await assert.rejects(nativeLookup(restored, { kind: 'recall', query: 'staging' }), /database changed/);
  assert.equal(await verifyNativeMemory(memory), true);
});


integration('real native post-capture premise links use follows-from without supersession and reject uncontrolled grounding', async t => {
  const { root, nativeConfig, memory } = await fixture(t);
  const updated = await applyNativeMemory(memory, { operations: [
    { tool: 'capture_decision', key: 'measurement-reading', sourceIds: ['src-1'], args: {
      title: 'Queue measurements support a bounded trial', rationale: 'Noah identified high queue wait together with normal DNS timing, supporting a bounded experiment.',
      topic_keys: ['queue'], options: [{ label: 'Test the bounded trial' }], still_proposed: true,
      grounding: [{ kind: 'evidence', evidence_id: 'measurements' }], binds: { level: 'nothing' }, undo_cost: { level: 'cheap' },
    } },
    { tool: 'ground_decision', sourceIds: ['src-1'], args: { decision_id: 'pilot', grounding: [{ kind: 'decision', decision_id: 'measurement-reading' }] } },
  ] }, { nativeConfig, checkpointRoot: join(root, 'premise'), revision: 'premise', eligibleSources: sources.slice(0, 2), sourceCutoff: 2 });
  const view = await createNativeTaskView(updated, { nativeConfig, taskRoot: join(root, 'premise-task') });
  const result = await nativeLookup(view, { kind: 'expand', id: 'pilot' });
  const premise = updated.records.find(r => r.key === 'measurement-reading');
  assert.ok(result.recordIds.includes(premise.id));
  assert.match(JSON.stringify(result.payload.native), /FOLLOWS_FROM|follows_from|follows-from/);
  assert.ok(!JSON.stringify(result.payload.native).includes('"status":"superseded"'));
  const audit = JSON.parse(await readFile(join(root, 'premise', 'capture.json'), 'utf8'));
  assert.equal(audit.results.at(-1).result.transport, 'allowlisted-native-cli-event-primitives');
  assert.deepEqual(audit.results.at(-1).result.nativeEvents[0].argv.slice(0, 4), ['emit', 'relation.added', '--kind', 'follows-from']);
  await assert.rejects(applyNativeMemory(updated, { operations: [
    { tool: 'ground_decision', sourceIds: ['src-1'], args: { decision_id: 'pilot', grounding: [{ kind: 'assumption', statement: 'Invented automatic premise' }] } },
  ] }, { nativeConfig, checkpointRoot: join(root, 'bad-ground'), revision: 'bad-ground', eligibleSources: sources.slice(0, 2), sourceCutoff: 2 }), /only retained evidence or decision IDs/);
  assert.equal(await verifyNativeMemory(memory), true); assert.equal(await verifyNativeMemory(updated), true);
});


integration('real native query echo cannot unlock historical artifacts; delivered record content can', async t => {
  const root = await mkdtemp(join(tmpdir(), 'v5-native-artifact-gate-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const nativeConfig = await inspectNativeExecutable({ executable: binary });
  const original = buildCorpus().find(s => s.id === selectedScreenIds[0]);
  const acquisition = createWorld(original, { phase: 'acquisition' });
  for (const action of validAcquisitionPlan(original)) applyAction(acquisition, action);
  assert.equal(acquisition.status, 'complete');
  const scenario = bindAcquisition(original, acquisition).scenario;
  const visible = scenario.episodes.flatMap(e => e.sources);
  const artifactId = scenario.worldDefinition.reuse[0].artifactId;
  for (const retained of [false, true]) {
    const memory = await applyNativeMemory(null, { operations: [{ tool: 'capture_decision', key: 'handoff', sourceIds: [visible[0].id], args: {
      title: 'Bounded staging handoff', rationale: retained
        ? `The previous check completed as artifact ${artifactId}. Keep current work within the documented scope.`
        : 'The staging handoff preserves limited scope but did not retain any previous artifact identifiers.',
      topic_keys: ['staging'], options: [{ label: 'Bounded continuation' }], still_proposed: true,
      grounding: [{ kind: 'assumption', statement: 'Current work must respect its scope.' }], binds: { level: 'nothing' }, undo_cost: { level: 'cheap' },
    } }] }, { nativeConfig, checkpointRoot: join(root, `memory-${retained}`), revision: `artifact-${retained}`,
      eligibleSources: visible, sourceCutoff: Math.max(...visible.map(s => s.order)) });
    let calls = 0;
    const task = { id: `artifact-gate-${retained}`, kind: 'action', mode: 'ordinary', control: 'primary', arm: 'loomtracer-native' };
    const result = await performWorkflow({ scenario, task, acquisition, memory, sources: visible, nativeConfig,
      taskRoot: join(root, `task-${retained}`), save: async () => {}, obtain: async () => {
        const value = ++calls === 1 ? { op: 'lookup', kind: 'recall', query: retained ? 'bounded staging handoff' : artifactId, limit: 1 }
          : calls === 2 ? { op: 'reuse', artifactId } : { op: 'message', text: 'Diagnostic only.' };
        return { value, call: { status: 'completed', usage: { input_tokens: 1, output_tokens: 1, total_tokens: 2 }, answer: JSON.stringify(value) } };
      } });
    assert.equal(result.status, 'operation-limit'); assert.equal(result.operationsUsed, 16);
    assert.equal(result.trace[0].receipt.ok, true);
    assert.ok(result.view.deliveredEvidence.some(e => e.kind === 'native' && e.text.includes(artifactId)), 'Both the query echo and actual record can contain the literal identifier');
    assert.equal(result.trace[1].receipt.ok, retained);
    assert.equal(result.world.reused.length, retained ? 1 : 0);
    if (!retained) assert.equal(result.trace[1].receipt.error?.code ?? result.world.errors[0]?.code, 'unretained_artifact');
    assert.equal(await verifyNativeMemory(memory), true);
  }
});
