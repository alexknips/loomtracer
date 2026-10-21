import test from 'node:test';
import assert from 'node:assert/strict';
import { MEMORY_VERSION, maintenancePrompt, applyMemoryEdits, createMemoryView, lookup, accountHistoricalDelivery,
  validateCitation, verifyMemoryRevision, verifyMemoryView } from './memory.mjs';
import { LIMITS } from './common.mjs';

const sources = [
  { id: 's1', text: 'Ada authorizes the relay only for the controlled pilot.', order: 1 },
  { id: 's2', text: 'Ben reports a new staffing limit; Ada has not withdrawn the pilot.', order: 2 },
  { id: 's3', text: 'Future evidence is controller-only.', order: 3 },
];
const first = () => applyMemoryEdits(null, { edits: [{ op: 'write', path: 'notes.md',
  text: '# Relay\nAda approved the controlled pilot only. [source:s1]\n' }] }, { eligibleSources: sources, revision: 'r1', sourceCutoff: 1 });
const deliver = (view, packet, id) => accountHistoricalDelivery(view, JSON.stringify(packet.payload), { id, packets: [packet] });

test('selective edits preserve parent bytes and immutable revision chain', () => {
  const one = first(), saved = JSON.stringify(one);
  const two = applyMemoryEdits(one, { edits: [
    { op: 'replace', path: 'notes.md', oldText: 'controlled pilot only.', text: 'controlled pilot only; no general adoption.' },
    { op: 'append', path: 'notes.md', text: '\nBen changed the staffing premise; no withdrawal is established. [source:s2]' },
    { op: 'write', path: 'work/temporary.md', text: 'Temporary scratch note' },
  ] }, { eligibleSources: sources, revision: 'r2', sourceCutoff: 2 });
  const three = applyMemoryEdits(two, { edits: [{ op: 'delete', path: 'work/temporary.md' }] }, { eligibleSources: sources, revision: 'r3', sourceCutoff: 2 });
  assert.equal(JSON.stringify(one), saved); assert.equal(two.parentHash, one.sha256); assert.equal(three.parentHash, two.sha256);
  assert.ok(!Object.hasOwn(three.files, 'work/temporary.md')); assert.equal(verifyMemoryRevision(three), true);
  assert.throws(() => applyMemoryEdits(one, { edits: [{ op: 'replace', path: 'notes.md', oldText: 'not present', text: 'x' }] }, { eligibleSources: sources, revision: 'r2' }), /exactly once/);
});

test('maintenance gets only prior notes and new messages, never archive or hidden episode keys', () => {
  const prompt = maintenancePrompt(first(), { messages: [sources[1]], hiddenAnswer: 'SECRET_ORACLE' });
  assert.ok(prompt.includes(sources[1].text)); assert.ok(!prompt.includes(sources[0].text));
  assert.ok(!prompt.includes(sources[2].text)); assert.ok(!prompt.includes('SECRET_ORACLE'));
});

test('future references, unsafe paths and invisible prior sources are rejected', () => {
  for (const text of ['Future [source:s3]', 'Unknown [source:missing]']) assert.throws(() => applyMemoryEdits(null, { edits: [{ op: 'write', path: 'x.md', text }] }, { eligibleSources: sources, revision: 'r1', sourceCutoff: 1 }), /future source/);
  for (const path of ['/absolute.md', '../escape.md', 'sub/../escape.md', 'script.js']) assert.throws(() => applyMemoryEdits(null, { edits: [{ op: 'write', path, text: 'x' }] }, { eligibleSources: sources, revision: 'r1' }), /relative Markdown/);
  assert.throws(() => applyMemoryEdits(first(), { edits: [] }, { eligibleSources: [sources[1]], revision: 'r2' }), /visibility/);
});

test('source expansion requires delivered retained references; pending lookup does not unlock it', () => {
  const view = createMemoryView(first(), sources);
  deliver(view, view.initialPacket, 'initial');
  assert.throws(() => lookup(view, { op: 'source', id: 's1' }), /already delivered/);
  const notes = lookup(view, { op: 'open', path: 'notes.md', startLine: 1, endLine: 2 });
  assert.throws(() => lookup(view, { op: 'source', id: 's1' }), /already delivered/);
  deliver(view, notes, 'notes');
  assert.equal(validateCitation(view, notes.evidence[0].id).valid, true);
  assert.equal(validateCitation(view, 'source:s1').valid, false);
  const source = lookup(view, { op: 'source', id: 's1' }); deliver(view, source, 'source');
  assert.equal(validateCitation(view, 'source:s1').valid, true);
  assert.throws(() => lookup(view, { op: 'source', id: 's3' }), /already delivered/);
});

test('immutable line citations validate only delivered ranges and revision', () => {
  const view = createMemoryView(first(), sources), packet = lookup(view, { op: 'open', path: 'notes.md', startLine: 2, endLine: 2 });
  deliver(view, packet, 'notes');
  assert.equal(validateCitation(view, { revision: 'r1', path: 'notes.md', startLine: 2, endLine: 2 }).valid, true);
  for (const citation of ['note:r2:notes.md:L2-L2', 'note:r1:notes.md:L1-L2', 'note:r1:notes.md:L2-L9', 'source:s2', 'nonsense']) assert.equal(validateCitation(view, citation).valid, false);
});

test('independent forks and serialized continuation preserve only their own delivered evidence', () => {
  const revision = first(), a = createMemoryView(revision, sources), b = createMemoryView(revision, sources);
  deliver(a, lookup(a, { op: 'open', path: 'notes.md' }), 'a');
  assert.ok(a.eligibleSourceIds.includes('s1')); assert.deepEqual(b.eligibleSourceIds, []);
  const restored = JSON.parse(JSON.stringify(a)); assert.equal(verifyMemoryView(restored), true);
  assert.deepEqual(lookup(restored, { op: 'source', id: 's1' }), lookup(a, { op: 'source', id: 's1' }));
  assert.equal(validateCitation(b, a.deliveredEvidence[0].id).valid, false);
});

test('repeated historical replay counts again; delivery ledger is bytes not token usage', () => {
  const view = createMemoryView(first(), sources), packet = lookup(view, { op: 'open', path: 'notes.md' });
  const a = deliver(view, packet, 'one'), b = deliver(view, packet, 'two');
  assert.equal(view.historicalBytes, a.bytes + b.bytes); assert.equal(view.deliveredEvidence.length, 1);
  assert.throws(() => deliver(view, packet, 'two'), /Unique/);
  assert.throws(() => accountHistoricalDelivery(view, 'x'.repeat(LIMITS.historicalPerTask + 1), { id: 'over' }), /32 KiB/);
  assert.equal(view.deliveries.length, 2);
});

test('undelivered, altered, or other-checkpoint packets cannot promote evidence', () => {
  const view = createMemoryView(first(), sources), packet = lookup(view, { op: 'open', path: 'notes.md' });
  assert.throws(() => accountHistoricalDelivery(view, '{}', { id: 'hidden', packets: [packet] }), /actually delivered/);
  const changed = structuredClone(packet); changed.payload.passages[0].text = 'tampered';
  assert.throws(() => deliver(view, changed, 'changed'), /integrity/);
  assert.throws(() => accountHistoricalDelivery(view, JSON.stringify([packet.payload, changed.payload]), { id: 'partial', packets: [packet, changed] }), /integrity/);
  assert.deepEqual(view.eligibleSourceIds, []); assert.deepEqual(view.deliveredEvidence, []); assert.equal(view.historicalBytes, 0);
  const other = createMemoryView(first(), sources, { sourceCutoff: 2 });
  assert.throws(() => deliver(other, packet, 'wrong'), /another checkpoint/);
  view.frozen.revision.files['notes.md'] = 'changed'; assert.throws(() => verifyMemoryView(view), /integrity/);
});

test('oracle and withheld controls do not inherit notes; withheld sources cannot be retrieved', () => {
  const oracle = createMemoryView(first(), sources, { mode: 'oracle', sourceCutoff: 2 });
  const withheld = createMemoryView(first(), sources, { mode: 'withheld', sourceCutoff: 2, withheldIds: ['s1'] });
  assert.equal(oracle.frozen.revision, null); assert.equal(withheld.frozen.revision, null);
  assert.deepEqual(withheld.initialPacket.payload.sources.map(s => s.id), ['s2']);
  deliver(withheld, withheld.initialPacket, 'initial');
  assert.throws(() => lookup(withheld, { op: 'source', id: 's1' }), /already delivered/);
  assert.throws(() => lookup(withheld, { op: 'open', path: 'notes.md' }), /no hidden notes/);
  assert.equal(validateCitation(withheld, 'source:s1').valid, false);
  assert.equal(validateCitation(withheld, 'source:s2').valid, true);
});

test('search returns genuine source ranges and empty searches provide no invented evidence', () => {
  const view = createMemoryView(first(), sources);
  const found = lookup(view, { op: 'search', query: 'controlled pilot' });
  assert.equal(found.evidence.length, 1); assert.ok(found.evidence[0].sourceIds.includes('s1'));
  assert.equal(lookup(view, { op: 'search', query: 'nonexistent' }).evidence.length, 0);
  assert.throws(() => lookup(view, { op: 'open', path: 'notes.md', startLine: 0 }), /Invalid/);
});

test('index headings are actual delivered passages with immutable references', () => {
  const revision = applyMemoryEdits(null, { edits: [{ op: 'write', path: 'notes.md', text: '# Pilot only [source:s1]\nDetails elsewhere.' }] }, { eligibleSources: sources, revision: 'r1', sourceCutoff: 1 });
  const view = createMemoryView(revision, sources); deliver(view, view.initialPacket, 'initial');
  assert.equal(validateCitation(view, 'note:r1:notes.md:L1-L1').valid, true);
  assert.ok(lookup(view, { op: 'source', id: 's1' }).payload.source);
  assert.equal(validateCitation(view, 'note:r1:notes.md:L2-L2').valid, false);
});

const jsonBytes = value => Buffer.byteLength(JSON.stringify(value), 'utf8');
const textView = text => createMemoryView(applyMemoryEdits(null,
  { edits: [{ op: 'write', path: 'notes.md', text }] },
  { eligibleSources: [], revision: 'r1' }), []);

test('lookup correction is separately versioned and preserves the existing allowances', () => {
  assert.equal(MEMORY_VERSION, 'functionality-v5-selective-markdown-1.1.0');
  assert.equal(LIMITS.lookupBytes, 8192);
  assert.equal(LIMITS.historicalPerTask, 32768);
});

test('incident-sized 51-line notes permit half-file payloads without charging the private evidence copy', () => {
  // A standalone synthetic fixture approximates the observed long-line handoff;
  // tests never depend on or modify a private live-run artifact.
  const notes = Array.from({ length: 51 }, (_, i) => `Line ${String(i + 1).padStart(2, '0')}: retained incident observation. `.padEnd(158, 'x')).join('\n');
  const view = textView(notes);
  assert.equal(Buffer.byteLength(notes), 8108);
  for (const [startLine, endLine] of [[1, 26], [27, 51]]) {
    const packet = lookup(view, { op: 'open', path: 'notes.md', startLine, endLine });
    assert.ok(jsonBytes(packet.payload) < LIMITS.lookupBytes);
    assert.ok(jsonBytes(packet) > LIMITS.lookupBytes, 'The old whole-packet check would reject this valid delivery');
    assert.deepEqual(packet.evidence, packet.payload.passages, 'Controller evidence remains intact');
    const delivery = deliver(view, packet, `half-${startLine}`);
    assert.equal(delivery.bytes, jsonBytes(packet.payload));
    assert.equal(validateCitation(view, packet.evidence[0].id).valid, true);
  }
  assert.throws(() => lookup(view, { op: 'open', path: 'notes.md', startLine: 1, endLine: 51 }), /exceeds lookup allowance/);
});

test('open accepts exactly 8192 serialized UTF-8 payload bytes and rejects one additional byte', () => {
  const overhead = jsonBytes(lookup(textView(''), { op: 'open', path: 'notes.md' }).payload);
  const remaining = LIMITS.lookupBytes - overhead;
  const note = 'é'.repeat(Math.floor(remaining / 2)) + 'x'.repeat(remaining % 2);
  const packet = lookup(textView(note), { op: 'open', path: 'notes.md' });
  assert.equal(jsonBytes(packet.payload), LIMITS.lookupBytes);
  assert.ok(jsonBytes(packet) > LIMITS.lookupBytes);
  assert.throws(() => lookup(textView(`${note}x`), { op: 'open', path: 'notes.md' }), /exceeds lookup allowance/);
});

test('source expansion measures its delivered source payload and preserves reference eligibility', () => {
  const makeSourceView = text => {
    const source = { id: 's1', text, order: 1 };
    const revision = applyMemoryEdits(null, { edits: [{ op: 'write', path: 'notes.md', text: '# Linked observation [source:s1]' }] }, { eligibleSources: [source], revision: 'r1' });
    const view = createMemoryView(revision, [source]);
    assert.throws(() => lookup(view, { op: 'source', id: 's1' }), /already delivered/);
    deliver(view, view.initialPacket, 'initial');
    return view;
  };
  const overhead = jsonBytes(lookup(makeSourceView(''), { op: 'source', id: 's1' }).payload);
  const sourceText = 'x'.repeat(LIMITS.lookupBytes - overhead);
  const view = makeSourceView(sourceText), packet = lookup(view, { op: 'source', id: 's1' });
  assert.equal(jsonBytes(packet.payload), LIMITS.lookupBytes);
  assert.ok(jsonBytes(packet) > LIMITS.lookupBytes);
  assert.equal(packet.evidence[0].text, sourceText);
  deliver(view, packet, 'source');
  assert.equal(validateCitation(view, 'source:s1').valid, true);
  assert.throws(() => lookup(makeSourceView(`${sourceText}x`), { op: 'source', id: 's1' }), /exceeds lookup allowance/);
});

test('search packs all matching passages that fit the public payload, not duplicated audit storage', () => {
  const text = ['needle first', 'a'.repeat(1450), 'b'.repeat(1450), 'needle second', 'c'.repeat(1450), 'd'.repeat(1450)].join('\n');
  const view = textView(text), packet = lookup(view, { op: 'search', query: 'needle' });
  assert.equal(packet.payload.passages.length, 2, 'Both matching results fit one participant payload');
  assert.ok(jsonBytes(packet.payload) <= LIMITS.lookupBytes);
  assert.ok(jsonBytes(packet) > LIMITS.lookupBytes);
  assert.deepEqual(packet.evidence.map(p => p.startLine), [1, 3]);
  deliver(view, packet, 'search');
  assert.equal(view.historicalBytes, jsonBytes(packet.payload));
  deliver(view, packet, 'replayed-search');
  assert.equal(view.historicalBytes, 2 * jsonBytes(packet.payload), 'Cumulative replay policy is unchanged');
});

test('initial oracle delivery admits its actual historical container even when audit storage exceeds 32 KiB', () => {
  const view = createMemoryView(null, [{ id: 's1', text: 'Observation. '.repeat(1700), order: 1 }], { mode: 'oracle' });
  const packet = view.initialPacket, historicalText = JSON.stringify([packet.payload]);
  assert.ok(Buffer.byteLength(historicalText) < LIMITS.historicalPerTask);
  assert.ok(jsonBytes(packet) > LIMITS.historicalPerTask, 'The old full-envelope gate would reject this valid initial delivery');
  assert.equal(validateCitation(view, 'source:s1').valid, false, 'Admission does not itself deliver evidence');
  accountHistoricalDelivery(view, historicalText, { id: 'initial', packets: [packet] });
  assert.equal(view.historicalBytes, Buffer.byteLength(historicalText));
  assert.equal(validateCitation(view, 'source:s1').valid, true);
});

test('initial admission includes array framing at the exact 32 KiB UTF-8 boundary', () => {
  const make = text => createMemoryView(null, [{ id: 's1', text, order: 1 }], { mode: 'oracle' });
  const overhead = jsonBytes([make('').initialPacket.payload]);
  const remaining = LIMITS.historicalPerTask - overhead;
  const text = 'é'.repeat(Math.floor(remaining / 2)) + 'x'.repeat(remaining % 2);
  const view = make(text), packet = view.initialPacket;
  assert.equal(jsonBytes([packet.payload]), LIMITS.historicalPerTask);
  assert.equal(jsonBytes(packet.payload), LIMITS.historicalPerTask - 2, 'Array framing consumes two actual delivered bytes');
  assert.ok(jsonBytes(packet) > LIMITS.historicalPerTask);
  accountHistoricalDelivery(view, JSON.stringify([packet.payload]), { id: 'initial', packets: [packet] });
  assert.equal(view.historicalBytes, LIMITS.historicalPerTask);
  accountHistoricalDelivery(view, JSON.stringify([packet.payload]), { id: 'replay', packets: [packet] });
  assert.equal(view.historicalBytes, 2 * LIMITS.historicalPerTask);
  assert.equal(view.uniqueExposureBytes, LIMITS.historicalPerTask - 2);
  assert.equal(view.deliveries.length, 2);
  assert.throws(() => make(`${text}x`), /Initial packet exceeds/);
});
