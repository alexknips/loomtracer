import test from 'node:test';
import assert from 'node:assert/strict';
import { scoreH, scoreAction, citationCompliance, ratings, firstObservedLoss, DIMENSIONS } from './scoring.mjs';
const units = DIMENSIONS.flatMap((dimension, n) => [{ id: `unit-${n}`, dimension, dependsOn: n === 1 ? ['unit-0'] : [] }]);
const pass = id => ({ id, status: 'pass', supported: true, completeUnambiguous: true, materialContradiction: false });
const scenario = (identity = 'same') => ({ id: `case-${identity}`, level: 'demanding', family: identity, hUnits: units,
  qUnits: [{ id: 'q-needed', kind: 'necessary' }, { id: 'q-known', kind: 'avoid' }, { id: 'q-guard', kind: 'avoid', guard: true }],
  wUnits: [{ id: 'w-reuse', kind: 'reuse' }, { id: 'w-guard', kind: 'reuse', guard: true }, { id: 'w-fresh', kind: 'fresh' }],
  identityEvidence: { classification: identity } });
const actionGrade = () => ({ qUnits: ['q-needed', 'q-known', 'q-guard'].map(pass), wUnits: ['w-reuse', 'w-guard', 'w-fresh'].map(pass),
  identity: { ...pass('identity'), observed: true, late: false }, completion: { status: 'pass' }, unsupportedClaims: [] });

test('strict incomplete unit fails locally while granular recovery remains useful', () => {
  const j = units.map(u => pass(u.id)); j[0].incomplete = true;
  const r = scoreH(units, j);
  assert.equal(r.rating, 75); assert.equal(r.completeChain, 'fail'); assert.equal(r.units[1].status, 'pass');
});
test('material false premise invalidates affected dependencies only', () => {
  const j = units.map(u => pass(u.id)); j[0].materialContradiction = true;
  const r = scoreH(units, j);
  assert.equal(r.rating, 50); assert.equal(r.units[1].status, 'fail'); assert.equal(r.units[2].status, 'pass');
});
test('misattribution, fabricated withdrawal and correction-as-reversal remain strict failures', () => {
  for (const [index, name] of [[2, 'misattribute-adoption'], [3, 'fabricate-withdrawal'], [3, 'extraction-correction-as-reversal']]) {
    const j = units.map(u => pass(u.id)); j[index] = { ...j[index], status: 'fail', reason: name, materialContradiction: true };
    const r = scoreH(units, j); assert.equal(r.completeChain, 'fail'); assert.equal(r.rating, 75);
  }
});
test('invented material extra claim blocks complete chain and only mapped units', () => {
  const r = scoreH(units, units.map(u => pass(u.id)), { unsupportedClaims: [{ material: true, claim: 'Invented commitment', affectedUnitIds: ['unit-2'] }] });
  assert.equal(r.rating, 75); assert.equal(r.completeChain, 'fail'); assert.equal(r.unsupportedClaims.length, 1);
});
test('invalid citation is separate from semantic H and future reference fails citation visibility', () => {
  const H = scoreH(units, units.map(u => pass(u.id)));
  const c = citationCompliance(['old', 'future', 'invented'], [{ id: 'old', order: 1 }, { id: 'future', order: 3 }], { cutoff: 2 });
  assert.equal(H.rating, 100); assert.equal(c.valid, 1); assert.equal(c.rows[1].error, 'future-reference'); assert.equal(c.rows[2].error, 'missing-reference');
  assert.equal(citationCompliance([], []).rating, null);
});
test('pending units keep bounds and missing dimensions prevent a rating', () => {
  const r = scoreH(units, [pass('unit-0')]); assert.equal(r.rating, null); assert.equal(r.lower, 25); assert.equal(r.upper, 100);
  assert.equal(scoreH(units.slice(0, 2), units.slice(0, 2).map(u => pass(u.id))).rating, null);
});
test('unnecessary and inactive-branch questions fail avoid side without unbounded penalty', () => {
  const g = actionGrade(); g.qUnits[1].status = 'fail';
  const r = scoreAction(scenario(), g); assert.equal(r.Q.rating, 67); assert.equal(r.completeTask, 'fail');
  g.qUnits[0].status = 'fail'; assert.equal(scoreAction(scenario(), g).Q.rating, 0);
});
test('stale reuse, skipped fresh work and recognized-but-repeated work fail W independently of D', () => {
  for (const id of ['w-reuse', 'w-fresh', 'w-guard']) {
    const g = actionGrade(); g.wUnits.find(u => u.id === id).status = 'fail';
    const r = scoreAction(scenario(), g); assert.equal(r.D.status, 'pass'); assert.ok(r.W.rating < 100); assert.equal(r.completeTask, 'fail');
  }
});
test('false, missed, unobserved and late D never get recognition credit', () => {
  for (const identity of [{ status: 'fail', category: 'false-link' }, { status: 'fail', category: 'missed-match' }]) {
    const g = actionGrade(); g.identity = { ...g.identity, ...identity }; assert.equal(scoreAction(scenario(), g).D.status, 'fail');
  }
  const g = actionGrade();
  const late = scoreAction(scenario(), g, { deterministic: { D: { late: true, observed: true } } }); assert.equal(late.D.status, 'fail');
  const ordinary = scoreAction(scenario(), g, { deterministic: { D: { observed: false } }, mode: 'ordinary' });
  const prompted = scoreAction(scenario(), g, { deterministic: { D: { observed: false } }, mode: 'prompted' });
  assert.equal(ordinary.completeTask, 'pass'); assert.equal(prompted.completeTask, 'fail');
});
test('semantic late recognition stays visible when the structural detector reports false', () => {
  const g = actionGrade();
  g.identity.late = true;
  const before = structuredClone(g);
  const r = scoreAction(scenario(), g, { deterministic: { D: { observed: true, late: false } } });
  assert.equal(r.D.status, 'fail');
  assert.equal(r.D.late, true);
  assert.equal(r.D.category, 'late');
  assert.deepEqual(g, before, 'Display correction must not rewrite the semantic judgment');
});
test('either timing source can establish late recognition and timely recognition remains timely', () => {
  for (const [structural, semantic] of [[true, false], [false, true], [true, true], [false, false]]) {
    const g = actionGrade(); g.identity.late = semantic;
    const r = scoreAction(scenario(), g, { deterministic: { D: { observed: true, late: structural } } });
    assert.equal(r.D.late, structural || semantic);
    assert.equal(r.D.status, structural || semantic ? 'fail' : 'pass');
  }
});
test('three demanding cases can score a diagnostic without fabricating full-suite coverage', () => {
  const scenarios = ['same', 'distinct', 'unresolved'].map(scenario);
  const results = scenarios.flatMap(s => [
    { scenarioId: s.id, kind: 'H', grade: { hUnits: units.map(u => pass(u.id)) }, status: 'completed' },
    { scenarioId: s.id, kind: 'action', mode: 'ordinary', grade: actionGrade(), status: 'completed' }
  ]);
  const r = ratings({ scenarios, results, mode: 'ordinary' });
  assert.equal(r.perLevel.demanding.components.H.rating, 100); assert.equal(r.perLevel.demanding.components.D.rating, 100);
  assert.equal(r.perLevel.demanding.overall.rating, 100); assert.equal(r.overall.rating, null);
  assert.equal(ratings({ scenarios: scenarios.slice(0, 2), results: results.slice(0, 4), mode: 'ordinary' }).perLevel.demanding.components.D.rating, null);
});
test('first loss requires observed evidence at every preceding stage', () => {
  const present = { status: 'present', evidence: ['saved-pointer'] }, absent = { status: 'absent', evidence: ['searched-snapshot'] };
  assert.equal(firstObservedLoss({ acquired: present, retained: absent }).stage, 'maintenance/capture');
  assert.equal(firstObservedLoss({ acquired: { status: 'unknown' }, retained: absent }).stage, 'not established');
  assert.equal(firstObservedLoss({ acquired: present, retained: present, delivered: present, reconstructed: absent }).stage, 'answer reconstruction');
});
