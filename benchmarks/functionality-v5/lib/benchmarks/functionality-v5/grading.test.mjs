import test from 'node:test';
import assert from 'node:assert/strict';
import { buildHPackets, buildActionPacket, buildWithheldPacket, validateGrade, mergeHGrades, qualificationFixture, qualifyGrader, humanAuditSelection, gradePrompt } from './grading.mjs';
import { DIMENSIONS } from './scoring.mjs';
import { buildCorpus } from './corpus.mjs';
import { publicWorldContract } from './world.mjs';
const scenario = { id: 'arm-secret-case', hUnits: Array.from({ length: 20 }, (_, i) => ({ id: `u${i}`, dimension: DIMENSIONS[Math.floor(i / 5)], criterion: 'Retain the stated fact with scope.', sourceIds: ['s1'], dependsOn: i === 11 ? ['u0'] : [] })),
  qUnits: [{ id: 'q1', kind: 'necessary', criterion: 'Ask unresolved field.' }], wUnits: [{ id: 'w1', kind: 'fresh', criterion: 'Execute required check.' }], identityEvidence: { classification: 'same', sourceIds: ['s1'] } };
const workflow = { id: 'secret-arm-workflow', kind: 'H', mode: 'prompted', status: 'completed', arm: 'secret-arm', model: 'secret-model',
  answer: { text: 'The approved scope is the pilot only.' }, trace: [], world: { secretOracle: 'must-not-be-participant-evidence' } };
const sources = [{ id: 's1', text: 'The scope is the pilot only.' }];
const judgment = id => ({ id, status: 'pass', completeUnambiguous: true, supported: true, materialContradiction: false, reason: 'Complete scoped assertion.',
  answerEvidence: [{ pointer: '/participant/answer/text', quote: 'pilot only' }], sourceIds: ['s1'], absenceReviewed: false });
const grade = packet => ({ blindId: packet.blindId, hUnits: (packet.sourceTruth.hUnits ?? []).map(u => judgment(u.id)),
  qUnits: (packet.sourceTruth.qUnits ?? []).map(u => judgment(u.id)), wUnits: (packet.sourceTruth.wUnits ?? []).map(u => judgment(u.id)),
  controlUnits: (packet.sourceTruth.controlUnits ?? []).map(u => judgment(u.id)), identity: packet.kind === 'action' ? { ...judgment('identity'), observed: true, late: false, category: null } : null,
  completion: packet.kind === 'action' ? { status: 'pass', reason: 'All public obligations fulfilled.' } : null, unsupportedClaims: [], uncertainties: [] });

test('H partitions have ten units, full answer/evidence, and no arm/model/raw world leak', () => {
  const packets = buildHPackets(scenario, workflow, sources);
  assert.equal(packets.length, 2); assert.equal(packets[0].sourceTruth.hUnits.length, 10);
  assert.deepEqual(packets[0].participant.answer, packets[1].participant.answer);
  assert.deepEqual(packets[0].sourceTruth.sources, sources);
  assert.ok(!JSON.stringify(packets).includes('secret-arm')); assert.ok(!JSON.stringify(packets).includes('secret-model'));
  assert.ok(!JSON.stringify(packets).includes('must-not-be-participant-evidence'));
});
test('missing or duplicate rubric judgments and invented reviewer evidence remain pending', () => {
  const packet = buildHPackets(scenario, workflow, sources)[0];
  for (const mutate of [
    g => g.hUnits.pop(),
    g => { g.hUnits[1].id = g.hUnits[0].id; },
    g => { g.hUnits[0].sourceIds = ['future-source']; },
    g => { g.hUnits[0].answerEvidence[0].quote = 'invented'; },
    g => { g.hUnits[0].answerEvidence[0].pointer = '/sourceTruth/sources/0/text'; }
  ]) { const g = grade(packet); mutate(g); assert.equal(validateGrade(packet, g).status, 'pending'); }
});
test('incomplete or ambiguous passes normalize to strict failure, not fuzzy credit', () => {
  const packet = buildHPackets(scenario, workflow, sources)[0], g = grade(packet);
  g.hUnits[0].completeUnambiguous = false;
  assert.equal(validateGrade(packet, g).grade.hUnits[0].status, 'fail');
});
test('cross-partition unsupported material claims invalidate only frozen dependencies', () => {
  const packets = buildHPackets(scenario, workflow, sources), grades = packets.map(grade);
  grades[1].unsupportedClaims.push({ claim: 'pilot only', material: true, affectedUnitIds: ['u0'], reason: 'Fixture marks this assertion contradicted.', answerEvidence: [{ pointer: '/participant/answer/text', quote: 'pilot only' }] });
  const r = mergeHGrades(scenario, packets, grades);
  assert.equal(r.status, 'model-reviewed'); assert.equal(r.score.units.find(u => u.id === 'u0').status, 'fail');
  assert.equal(r.score.units.find(u => u.id === 'u11').status, 'fail'); assert.equal(r.score.units.find(u => u.id === 'u12').status, 'pass');
});
test('changed packet and missing partition cannot be silently graded', () => {
  const packets = buildHPackets(scenario, workflow, sources), g = grade(packets[0]);
  packets[0].participant.answer.text = 'changed';
  assert.equal(validateGrade(packets[0], g).status, 'pending');
  assert.equal(mergeHGrades(scenario, packets, []).status, 'pending');
});
test('action evidence stays observed, structural checks distinct, ordinary D is visible mode', () => {
  const packet = buildActionPacket(scenario, { ...workflow, kind: 'action', mode: 'ordinary', deterministic: { D: { observed: false } } }, sources);
  assert.equal(packet.participant.mode, 'ordinary'); assert.equal(packet.structuralChecks.D.observed, false);
  assert.equal(validateGrade(packet, grade(packet)).status, 'model-reviewed');
});
test('action packets expose the exact public tool contract, task and consumed operation budget', () => {
  const s = buildCorpus().find(x => x.id === 'v5-experiment-demanding-distinct');
  const packet = buildActionPacket(s, { ...workflow, kind: 'action', operationsUsed: 16 }, s.episodes.flatMap(e => e.sources));
  assert.deepEqual(packet.sourceTruth.publicContract, publicWorldContract(s));
  assert.equal(packet.sourceTruth.publicTask, s.task);
  assert.deepEqual(packet.participant.execution, { operationsUsed: 16, operationLimit: 16 });
  assert.ok(packet.sourceTruth.publicContract.operations.find(x => x.operation === 'verify_operator').description.includes('both staffing and time of day'));
});
test('blinded review receives only actually delivered passages and first-delivery timing, not private view state', () => {
  const passages = [
    { id: 'source:s1', kind: 'source', text: 'The scope is the pilot only.', sourceIds: ['s1'], sourceId: 's1' },
    { id: 'note:r0:notes.md:L1-L1', kind: 'markdown', revision: 'r0', path: 'notes.md', startLine: 1, endLine: 1, text: 'Pilot only [source:s1]', sourceIds: ['s1'] },
    { id: 'native:r0:p2', kind: 'native', revision: 'r0', recordIds: ['record-1'], text: '{"scope":"pilot only"}', sourceIds: ['s1'], payloadHash: 'p2', path: '/secret/native-task', privateState: 'must-not-leak' },
  ];
  const w = { ...workflow, packets: [{ sha256: 'p0', evidence: [passages[0]] }, { sha256: 'p1', evidence: [passages[1]] }, { sha256: 'p2', evidence: [passages[2]] }, { sha256: 'pending', evidence: [{ id: 'never-delivered', text: 'future' }] }],
    view: { deliveredEvidence: passages, deliveries: [{ packetHashes: ['p0'] }, { packetHashes: ['p0', 'p1', 'p2'] }], frozen: { sources: [{ id: 'secret-archive' }] }, nativeTask: { workspace: '/secret/native-task', implementation: 'secret-arm' } } };
  const packet = buildHPackets(scenario, w, sources)[0];
  assert.equal(packet.participant.deliveredEvidence.length, 3);
  assert.deepEqual(packet.participant.deliveredEvidence.map(x => x.firstDeliveredInvocation), [1, 2, 2]);
  assert.equal(packet.participant.deliveredEvidence[1].path, 'notes.md');
  assert.equal(packet.participant.deliveredEvidence[2].path, undefined);
  for (const secret of ['/secret/native-task', 'must-not-leak', 'secret-archive', 'never-delivered', 'secret-arm']) assert.ok(!JSON.stringify(packet).includes(secret), secret);
});
test('review instructions distinguish early supported recognition from early ambiguity and prevent duplicate guard penalties', () => {
  const packet = buildActionPacket(scenario, { ...workflow, kind: 'action' }, sources);
  const prompt = gradePrompt(packet);
  assert.ok(prompt.includes('A late restatement does not erase an earlier supported decision'));
  assert.ok(prompt.includes('early ambiguous, incorrect or unsupported suggestion followed only by a correct decision after substantive work still FAILS as late'));
  assert.ok(prompt.includes('do not duplicate a penalty already assigned'));
  assert.ok(prompt.includes('Evidence requested by a lookup becomes available on the next invocation'));
});
test('withheld controls require regenerated expectations and omit original full H rubric', () => {
  assert.throws(() => buildWithheldPacket(scenario, [workflow], sources), /expectation/);
  const packet = buildWithheldPacket(scenario, [workflow], sources, { expectation: { H: 'Report the discriminator as unknown.', action: 'Do safe work and block dependent action.' } });
  assert.equal(packet.sourceTruth.hUnits, undefined); assert.equal(packet.sourceTruth.controlUnits.length, 1);
  assert.equal(packet.sourceTruth.controlUnits[0].criterion, 'Report the discriminator as unknown.');
});
test('fixed grader qualification requires strict pass and incomplete fail', () => {
  const { packet } = qualificationFixture();
  const g = { ...grade(packet), hUnits: packet.sourceTruth.hUnits.map(u => ({ ...judgment(u.id), sourceIds: ['fixture-source'], answerEvidence: [{ pointer: '/participant/answer/text', quote: 'pilot only' }] })) };
  g.hUnits[1] = { ...g.hUnits[1], status: 'fail', completeUnambiguous: false, reason: 'Missing staffing and the joint relationship.' };
  assert.equal(qualifyGrader(packet, g).passed, true);
  g.hUnits[1].status = 'pass'; g.hUnits[1].completeUnambiguous = true;
  assert.equal(qualifyGrader(packet, g).passed, false);
});
test('audit retains uncertain and consequential judgments beyond minimum quarter', () => {
  const packets = buildHPackets(scenario, workflow, sources);
  const grades = packets.map(p => ({ grade: grade(p) })); grades[1].grade.uncertainties = ['Requires human source interpretation.'];
  const audit = humanAuditSelection(packets, grades); assert.equal(audit.minimum, 1); assert.equal(audit.selected.length, 2); assert.equal(audit.status, 'pending-human-review');
});
test('every uncertain identity, completion and withheld judgment is audited even without a global uncertainty', () => {
  const packets = Array.from({ length: 8 }, (_, i) => ({ blindId: `audit-${i}`, kind: 'action', participant: { mode: 'ordinary' } }));
  const grades = packets.map(() => ({ grade: { hUnits: [], qUnits: [], wUnits: [], controlUnits: [], uncertainties: [], unsupportedClaims: [], identity: { status: 'pass' }, completion: { status: 'pass' } } }));
  grades[3].grade.identity.status = 'uncertain';
  grades[5].grade.completion.status = 'uncertain';
  grades[7].grade.controlUnits.push({ id: 'withheld-H', status: 'uncertain' });
  const audit = humanAuditSelection(packets, grades);
  assert.equal(audit.minimum, 2);
  assert.deepEqual(audit.selected.map(x => x.index), [0, 3, 5, 7]);
});
