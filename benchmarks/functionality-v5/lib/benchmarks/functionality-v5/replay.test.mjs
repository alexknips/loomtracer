import test from 'node:test';
import assert from 'node:assert/strict';
import { hash } from './common.mjs';
import { applyMemoryEdits, createMemoryView, lookup, accountHistoricalDelivery, replayPackets,
  migrateLegacyMemoryView, validateCitation, REPLAY_POLICY, REPLAY_POLICY_ID } from './memory.mjs';
import { migrateDeliveryLimitedWorkflow, performWorkflow } from './workflow.mjs';
import { buildCorpus, selectedScreenIds } from './corpus.mjs';

const copy = structuredClone;
const bytes = value => Buffer.byteLength(typeof value === 'string' ? value : JSON.stringify(value), 'utf8');
const legacyVersion = 'functionality-v5-selective-markdown-1.0.0';
function oldSeal(value) {
  const { sha256, ...body } = value;
  body.version = legacyVersion;
  return { ...body, sha256: hash(body) };
}
function oldView(revision, sources, options) {
  const view = createMemoryView(revision ? oldSeal(revision) : null, sources, options);
  view.initialPacket = oldSeal(view.initialPacket);
  view.version = legacyVersion;
  delete view.replayPolicyId; delete view.exposureLedger; delete view.uniqueExposureBytes;
  return view;
}
function oldDelivery(view, packets, id) {
  const text = JSON.stringify(packets.map(p => p.payload));
  const row = { id, bytes: bytes(text), textHash: hash(text), packetHashes: packets.map(p => p.sha256) };
  view.deliveries.push(row); view.historicalBytes += row.bytes;
  for (const packet of packets) for (const evidence of packet.evidence) {
    if (!view.deliveredEvidence.some(e => e.id === evidence.id)) view.deliveredEvidence.push(copy(evidence));
    for (const id of evidence.sourceIds) if (!view.eligibleSourceIds.includes(id)) view.eligibleSourceIds.push(id);
    if (evidence.kind === 'source' && !view.deliveredSourceIds.includes(evidence.sourceId)) view.deliveredSourceIds.push(evidence.sourceId);
  }
  return row;
}
function legacyCheckpoint() {
  const scenario = buildCorpus().find(s => s.id === selectedScreenIds[0]);
  const sources = [{ id: 's1', order: 1, text: 'é'.repeat(5300) }];
  const view = oldView(null, sources, { mode: 'oracle' });
  const id = `${scenario.id}-oracle-H`, packets = [view.initialPacket];
  const rows = [1, 2, 3].map(n => oldDelivery(view, packets, `${id}-step-${n}`));
  assert.ok(view.historicalBytes <= 32768);
  assert.ok(view.historicalBytes + rows[0].bytes > 32768);
  const state = { id, caseId: scenario.id, kind: 'H', mode: 'prompted', control: 'oracle', status: 'delivery-limit',
    world: null, view, packets, trace: rows.map((r, i) => ({ operation: { op: 'lookup', kind: 'search', query: 'history' }, receipt: { ok: false, error: 'Diagnostic source view has no hidden notes archive' }, index: i + 1 })),
    callIds: rows.map(r => r.id), operationsUsed: rows.length, answer: null, citations: [],
    errors: ['32 KiB cumulative historical delivery allowance exhausted'], deliveries: rows.map(r => ({ id: r.id, historicalBytes: r.bytes, promptHash: hash(r.id), packetHashes: r.packetHashes })) };
  return { scenario, sources, state };
}

test('repeated replay continues beyond 32 KiB while unique exposure and active context remain bounded', () => {
  const view = createMemoryView(null, [{ id: 's1', text: 'é'.repeat(7000), order: 1 }], { mode: 'oracle' });
  const packet = view.initialPacket, text = JSON.stringify([packet.payload]);
  for (let i = 0; i < 5; i++) accountHistoricalDelivery(view, text, { id: `delivery-${i}`, packets: [packet] });
  assert.equal(view.historicalBytes, bytes(text) * 5);
  assert.ok(view.historicalBytes > 32768);
  assert.equal(view.uniqueExposureBytes, bytes(packet.payload));
  assert.equal(view.exposureLedger.length, 1);
  assert.ok(view.deliveries.every(row => row.bytes <= 32768));
  assert.equal(view.replayPolicyId, REPLAY_POLICY_ID);
  assert.equal(REPLAY_POLICY.lookupBytes, 8192);
});
test('replay deduplicates only exact serialized payloads and preserves source/evidence bytes', () => {
  const revision = applyMemoryEdits(null, { edits: [{ op: 'write', path: 'n.md', text: '# Header\nFirst fact.\nSecond fact.' }] }, { eligibleSources: [], revision: 'r1' });
  const view = createMemoryView(revision, []), a = lookup(view, { op: 'open', path: 'n.md', startLine: 1, endLine: 2 }), b = lookup(view, { op: 'open', path: 'n.md', startLine: 2, endLine: 3 });
  const original = JSON.stringify([a, b]);
  const result = replayPackets(view, [a, copy(a), b, a]);
  assert.deepEqual(result, [a, b]);
  assert.equal(JSON.stringify([a, b]), original);
  accountHistoricalDelivery(view, JSON.stringify(result.map(p => p.payload)), { id: 'first', packets: result });
  assert.equal(view.uniqueExposureBytes, bytes(a.payload) + bytes(b.payload), 'Overlapping nonidentical ranges count conservatively');
});
test('distinct exposure admits exactly 32768 bytes but rejects new exposure atomically above it', () => {
  const empty = createMemoryView(applyMemoryEdits(null, { edits: [{ op: 'write', path: 'a.md', text: '' }] }, { eligibleSources: [], revision: 'r1' }), []);
  const overhead = bytes(lookup(empty, { op: 'open', path: 'a.md' }).payload);
  const text = 'é'.repeat(Math.floor((8192 - overhead) / 2)) + 'x'.repeat((8192 - overhead) % 2);
  const revision = applyMemoryEdits(null, { edits: ['a', 'b', 'c', 'd', 'e'].map(name => ({ op: 'write', path: `${name}.md`, text })) }, { eligibleSources: [], revision: 'r1' });
  const view = createMemoryView(revision, []);
  for (const name of ['a', 'b', 'c', 'd']) {
    const packet = lookup(view, { op: 'open', path: `${name}.md` });
    assert.equal(bytes(packet.payload), 8192);
    accountHistoricalDelivery(view, JSON.stringify(packet.payload), { id: name, packets: [packet] });
  }
  assert.equal(view.uniqueExposureBytes, 32768);
  const previous = JSON.stringify(view), extra = lookup(view, { op: 'open', path: 'e.md' });
  assert.throws(() => accountHistoricalDelivery(view, JSON.stringify(extra.payload), { id: 'e', packets: [extra] }), /distinct historical/);
  assert.equal(JSON.stringify(view), previous);
  assert.equal(validateCitation(view, extra.evidence[0].id).valid, false);
});
test('active serialized UTF-8 context includes array framing and cannot smuggle unregistered history', () => {
  const make = text => createMemoryView(null, [{ id: 's1', order: 1, text }], { mode: 'oracle' });
  const overhead = bytes([make('').initialPacket.payload]), left = 32768 - overhead;
  const view = make('é'.repeat(Math.floor(left / 2)) + 'x'.repeat(left % 2));
  const packet = view.initialPacket;
  accountHistoricalDelivery(view, JSON.stringify([packet.payload]), { id: 'exact', packets: [packet] });
  assert.throws(() => accountHistoricalDelivery(view, 'x'.repeat(32769), { id: 'too-large' }), /active historical/);
  assert.throws(() => accountHistoricalDelivery(view, JSON.stringify({ secret: 'unregistered history' }), { id: 'unregistered' }), /complete registered historical text/);
  assert.equal(view.deliveries.length, 1);
});
test('legacy migration preserves every original byte and reconstructs exposure from actual deliveries only', () => {
  const { state } = legacyCheckpoint(), original = JSON.stringify(state);
  const child = migrateDeliveryLimitedWorkflow(state);
  assert.equal(JSON.stringify(state), original);
  for (const field of ['trace', 'callIds', 'deliveries', 'errors', 'packets', 'citations']) assert.deepEqual(child[field], state[field]);
  assert.deepEqual(child.view.frozen, state.view.frozen);
  assert.deepEqual(child.view.deliveries, state.view.deliveries);
  assert.deepEqual(child.view.deliveredEvidence, state.view.deliveredEvidence);
  assert.equal(child.view.historicalBytes, state.view.historicalBytes);
  assert.equal(child.view.uniqueExposureBytes, bytes(state.packets[0].payload));
  assert.equal(child.replayMigration.fromStateHash, hash(state));
  assert.equal(child.status, 'running');
  assert.equal(validateCitation(child.view, 'source:s1').valid, true);
});
test('legacy pending note passages do not gain eligibility until actually delivered under the new policy', () => {
  const sources = [{ id: 's1', order: 1, text: 'Pilot only.' }];
  const revision = applyMemoryEdits(null, { edits: [{ op: 'write', path: 'n.md', text: '# Notes\nPilot only. [source:s1]' }] }, { eligibleSources: sources, revision: 'r1' });
  const old = oldView(revision, sources, { mode: 'notes' });
  oldDelivery(old, [old.initialPacket], 'step-1');
  const temporary = migrateLegacyMemoryView(old, [old.initialPacket]);
  const pending = oldSeal(lookup(temporary, { op: 'open', path: 'n.md' }));
  const migrated = migrateLegacyMemoryView(old, [old.initialPacket, pending]);
  assert.equal(migrated.uniqueExposureBytes, bytes(old.initialPacket.payload));
  assert.deepEqual(migrated.eligibleSourceIds, []);
  assert.throws(() => lookup(migrated, { op: 'source', id: 's1' }), /already delivered/);
  accountHistoricalDelivery(migrated, JSON.stringify([pending.payload]), { id: 'step-2', packets: [pending] });
  assert.equal(validateCitation(migrated, 'note:r1:n.md:L1-L2').valid, true);
  assert.equal(lookup(migrated, { op: 'source', id: 's1' }).payload.source.text, 'Pilot only.');
});
test('migration rejects altered packets, ledger hashes, invented visibility and unavailable future evidence', () => {
  const { state } = legacyCheckpoint();
  for (const mutate of [
    s => { s.packets[0].payload.sources[0].text = 'Tampered'; },
    s => { s.view.deliveries[0].textHash = 'wrong'; },
    s => { s.view.eligibleSourceIds.push('future'); },
    s => { s.view.deliveredSourceIds.push('future'); },
    s => { s.view.deliveredEvidence.push({ id: 'source:future' }); },
    s => { s.view.frozen.sources.push({ id: 'future', text: 'Hidden', order: 99 }); },
    s => { s.trace.pop(); },
    s => { s.callIds[0] = 'wrong-step'; },
  ]) {
    const altered = copy(state); mutate(altered);
    assert.throws(() => migrateDeliveryLimitedWorkflow(altered));
  }
});
test('completed, blocked, invalid and exhausted workflows are never reopened', () => {
  const { state } = legacyCheckpoint();
  for (const status of ['completed', 'complete', 'blocked', 'invalid-output', 'operation-limit', 'running', 'failed']) assert.throws(() => migrateDeliveryLimitedWorkflow({ ...state, status }), /Only an unfinished delivery-limit/);
  assert.throws(() => migrateDeliveryLimitedWorkflow({ ...state, operationsUsed: 16 }), /below 16/);
  assert.throws(() => migrateDeliveryLimitedWorkflow({ ...state, world: { status: 'blocked' } }), /Terminal world/);
});
test('resuming a migrated checkpoint calls only the next step and never replays prior operations', async () => {
  const { scenario, sources, state } = legacyCheckpoint(), original = JSON.stringify(state);
  const nextId = `${state.id}-step-${state.callIds.length + 1}`, obtained = [];
  const child = await performWorkflow({ scenario, task: { id: state.id, kind: 'H', mode: 'prompted', control: 'oracle' },
    sources, restore: migrateDeliveryLimitedWorkflow(state), save: async () => {}, obtain: async (id, prompt) => {
      obtained.push(id); assert.equal(id, nextId); assert.match(prompt, /cumulative replay bytes are not a stopping threshold/);
      const value = { op: 'answer', answers: [{ questionId: scenario.questions[0].id, text: 'Fixture answer only; not a semantic grade.', citations: ['source:s1'] }], uncertainty: [] };
      return { value, parseError: null, call: { status: 'completed', usage: { input_tokens: 1, output_tokens: 1, total_tokens: 2 }, answer: JSON.stringify(value) } };
    } });
  assert.deepEqual(obtained, [nextId]); assert.equal(child.status, 'completed');
  assert.equal(child.operationsUsed, state.operationsUsed + 1);
  assert.deepEqual(child.trace.slice(0, state.trace.length), state.trace);
  assert.deepEqual(child.callIds, [...state.callIds, nextId]);
  assert.ok(child.view.historicalBytes > 32768);
  assert.equal(child.view.uniqueExposureBytes, bytes(state.packets[0].payload));
  assert.equal(JSON.stringify(state), original);
  await performWorkflow({ scenario, task: { id: state.id, kind: 'H', mode: 'prompted', control: 'oracle' }, sources,
    restore: child, save: async () => {}, obtain: async () => { throw new Error('Completed workflows must not dispatch'); } });
});
