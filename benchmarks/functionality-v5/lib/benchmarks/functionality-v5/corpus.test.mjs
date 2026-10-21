import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { buildCorpus, bindAcquisition, eligibleSources, selectedScreenIds, H_DIMENSIONS } from './corpus.mjs';
import { createWorld, applyAction, validAcquisitionPlan, publicWorldContract } from './world.mjs';

const corpus = buildCorpus();
test('27 deterministic definitions cover three families, levels and identity conditions', () => {
  assert.equal(corpus.length, 27);
  assert.equal(new Set(corpus.map(x => x.id)).size, 27);
  assert.deepEqual(buildCorpus(), corpus);
  assert.equal(selectedScreenIds.filter(id => corpus.some(x => x.id === id)).length, 3);
});
test('strict H dimensions have 3/4/5 meaningful requirements and public question coverage', () => {
  for (const s of corpus) {
    const n = { easy: 3, intermediate: 4, demanding: 5 }[s.level];
    assert.equal(s.hUnits.length, n * 4);
    for (const d of H_DIMENSIONS) assert.equal(s.hUnits.filter(x => x.dimension === d).length, n);
    for (const h of s.hUnits) {
      assert.ok(s.questions.some(q => q.id === h.questionId));
      assert.ok(h.criterion.length > 40);
      assert.ok(h.sourceIds.length);
      for (const dependency of h.dependsOn) assert.ok(s.hUnits.some(x => x.id === dependency));
    }
  }
});
test('identity siblings have identical public current task and tool contracts', () => {
  for (const family of ['incident', 'experiment', 'release']) for (const level of ['easy', 'intermediate', 'demanding']) {
    const siblings = corpus.filter(s => s.family === family && s.level === level);
    assert.equal(new Set(siblings.map(x => x.task)).size, 1);
    assert.equal(new Set(siblings.map(x => JSON.stringify(x.contract))).size, 1);
    assert.equal(new Set(siblings.map(x => JSON.stringify(publicWorldContract(x)))).size, 1);
    for (const s of siblings) assert.ok(!JSON.stringify(publicWorldContract(s)).includes(s.id));
  }
});
test('difficulty changes actual required operations, dependency branches and noise', () => {
  for (const family of ['incident', 'experiment', 'release']) {
    const cases = ['easy', 'intermediate', 'demanding'].map(level => corpus.find(s => s.family === family && s.level === level && s.identity === 'same'));
    assert.deepEqual(cases.map(s => s.worldDefinition.fresh.length), [2, 3, 4]);
    assert.deepEqual(cases.map(s => s.episodes[1].sources.length), [8, 16, 24]);
    assert.equal(cases[0].worldDefinition.conditional, null);
    assert.ok(cases[2].worldDefinition.conditional);
    assert.notDeepEqual(cases[0].worldDefinition.fresh[0].prerequisites, cases[2].worldDefinition.fresh[0].prerequisites);
  }
});
test('four episodes and source occurrences preserve explicit actor and conversation identity', () => {
  for (const s of corpus) {
    assert.deepEqual(s.episodes.map(x => x.number), [1, 2, 3, 4]);
    const sources = s.episodes.flatMap(x => x.sources);
    assert.equal(new Set(sources.map(x => x.id)).size, sources.length);
    assert.ok(new Set(sources.map(x => x.actor)).size >= 6);
    assert.ok(new Set(sources.map(x => x.conversation)).size >= 6);
    assert.ok(sources.every(x => x.text && x.actor && x.conversation && !x.id.includes(s.identity)));
  }
});
test('acquisition binding records only actually executed results and never repairs a missing rationale', () => {
  const s = corpus.find(x => x.id === selectedScreenIds[0]);
  const a = createWorld(s, { phase: 'acquisition' });
  applyAction(a, { op: 'run', operation: 'pool_profile' });
  const { scenario: bound, acquisition } = bindAcquisition(s, a);
  assert.equal(acquisition.complete, false);
  assert.equal(acquisition.rationalePresent, false);
  assert.ok(bound.episodes[0].sources.some(x => x.id === 'acq-pool_profile'));
  assert.ok(!bound.episodes[0].sources.some(x => x.id === 'acq-dns_probe'));
  assert.ok(!bound.episodes[0].sources.some(x => x.id === 'origin-summary'));
  assert.deepEqual(bound.hUnits.map(x => x.criterion), s.hUnits.map(x => x.criterion));
  assert.equal(bound.hUnits.find(x => x.id === 'h-premises-2').sourceAvailability[0].available, false);
});
test('actual final summary is an exact-span alias of the retained original occurrence', () => {
  const s = corpus[0];
  const a = createWorld(s, { phase: 'acquisition' });
  const summary = 'Actual unedited originating summary.';
  const { scenario: bound } = bindAcquisition(s, a, { summary, traceSources: [{ id: 'origin-turn-1', actor: 'Originating agent', conversation: 'originating-work', text: `Before. ${summary} After.` }] });
  const alias = bound.episodes[0].sources.find(x => x.id === 'origin-summary');
  assert.equal(alias.aliasOf, 'origin-turn-1');
  const raw = bound.episodes[0].sources.find(x => x.id === alias.aliasOf);
  assert.equal(raw.text.slice(alias.sourceSpan.start, alias.sourceSpan.end), summary);
});
test('repeated acquisition checks remain distinct immutable source occurrences', () => {
  const s = corpus[0];
  const a = createWorld(s, { phase: 'acquisition' });
  applyAction(a, { op: 'run', operation: 'pool_profile' });
  applyAction(a, { op: 'run', operation: 'pool_profile' });
  const bound = bindAcquisition(s, a).scenario;
  const observations = bound.episodes[0].sources.filter(s => s.id.startsWith('acq-pool_profile'));
  assert.deepEqual(observations.map(s => s.id), ['acq-pool_profile', 'acq-pool_profile-occurrence-2']);
  assert.notEqual(observations[0].text, observations[1].text);
});
test('withheld release removes the decisive observation and dependent proof, not merely its citation', () => {
  const s = corpus.find(x => x.id === selectedScreenIds[2]);
  const a = createWorld(s, { phase: 'acquisition' });
  for (const action of validAcquisitionPlan(s)) applyAction(a, action);
  const summary = 'The old license passed and the manifest used it.';
  const bound = bindAcquisition(s, a, { summary, traceSources: [{ id: 'origin-turn-raw', actor: 'Originating agent', conversation: 'originating-work', text: summary }] }).scenario;
  const withheld = eligibleSources(bound, { withheld: true });
  for (const id of ['acq-validate_license', 'acq-build_original_manifest', 'origin-summary', 'origin-turn-raw']) assert.ok(!withheld.some(s => s.id === id));
  assert.ok(withheld.some(x => x.id === 'acq-validate_logo'));
  assert.ok(withheld.some(x => x.id === 'src-0011'));
});
test('known questions require actually using history, and dependent questions have frozen timing rules', () => {
  for (const s of corpus.filter(x => x.level === 'demanding')) {
    assert.ok(s.qUnits.some(x => x.kind === 'avoid' && Object.hasOwn(s.worldDefinition.knownFields, x.field)));
    assert.ok(s.qUnits.some(x => x.kind === 'necessary' && x.prerequisites.length));
    for (const u of s.wUnits.filter(x => x.kind === 'reuse' && !x.guard)) {
      assert.ok(s.worldDefinition.fresh.some(f => (s.identity !== 'unresolved' || !f.identityDependent) && f.prerequisites.includes(u.artifactId)));
    }
  }
});
test('shift contract correction preserves every frozen rubric unit and denominator', () => {
  const rubric = corpus.map(s => ({ id: s.id, hUnits: s.hUnits, qUnits: s.qUnits, wUnits: s.wUnits }));
  assert.equal(createHash('sha256').update(JSON.stringify(rubric)).digest('hex'), '42f58e7e73d302cbf770387a1e4cf06d1f94a1d880143422cdbc7b8f8403f3fc');
  for (const s of corpus.filter(s => s.family === 'experiment')) assert.equal(s.worldDefinition.knownFields.shift, 'one technician, daytime');
});
test('public operator contract requires both shift components without supplying their answer', () => {
  for (const s of corpus.filter(s => s.family === 'experiment' && s.level !== 'easy')) {
    const operation = publicWorldContract(s).operations.find(x => x.operation === 'verify_operator');
    assert.ok(operation.fields.includes('shift'));
    assert.match(operation.description, /both staffing and time of day together/);
    assert.match(operation.description, /values established in retained sources/);
    assert.match(operation.description, /one string.*either order/);
    assert.doesNotMatch(operation.description, /one technician|one-technician|daytime|nighttime/);
    assert.ok(s.episodes[2].sources.find(x => x.id === 'src-0012').text.includes(operation.description));
  }
});
