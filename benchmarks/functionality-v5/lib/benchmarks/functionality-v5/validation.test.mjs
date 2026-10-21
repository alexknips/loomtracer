import test from 'node:test';
import assert from 'node:assert/strict';
import { buildCorpus, bindAcquisition, eligibleSources } from './corpus.mjs';
import * as worldAdapter from './world.mjs';
import { validateCorpus } from './validation.mjs';

test('all 27 complete worlds satisfy authored requirements, valid alternative plans and reserve', () => {
  const r = validateCorpus(buildCorpus(), { worldAdapter });
  assert.equal(r.passed, true, JSON.stringify(r.checks.filter(c => !c.passed), null, 2));
  assert.equal(r.caseChecks.length, 27); assert.ok(r.checkCount >= 750);
  assert.ok(r.caseChecks.every(c => c.referencePlanOperations <= 12));
  assert.ok(r.caseChecks.every(c => c.oracleBytes <= 32768));
});
test('validation cannot certify an authored catalogue without world execution', () => {
  const r = validateCorpus(); assert.equal(r.passed, false);
  assert.ok(r.errors.includes('World transition and valid-plan adapter required for final validation'));
});
test('hidden extra H criteria, future source bindings, and public gold labels are rejected', () => {
  for (const mutate of [
    c => { c[0].hUnits[0].questionId = 'unasked-question'; },
    c => { c[0].hUnits[0].sourceIds.push('future-answer'); },
    c => { c[0].task += ' v5-incident-easy-same'; },
    c => { c[0].episodes[2].sources.push(structuredClone(c[0].episodes[0].sources[0])); }
  ]) { const corpus = buildCorpus(); mutate(corpus); assert.equal(validateCorpus(corpus, { worldAdapter }).passed, false); }
});
test('counterfactual siblings keep exactly the same current request but change identity evidence', () => {
  const siblings = buildCorpus().filter(s => s.family === 'experiment' && s.level === 'demanding');
  assert.equal(new Set(siblings.map(s => s.task)).size, 1);
  assert.equal(new Set(siblings.map(s => JSON.stringify(s.contract))).size, 1);
  assert.equal(new Set(siblings.map(s => s.episodes[2].sources.find(s => s.id === 'src-0011').text)).size, 3);
});
test('failed acquisition preserves obligations and marks unobserved results unavailable', () => {
  const scenario = buildCorpus().find(s => s.id === 'v5-incident-demanding-same');
  const world = worldAdapter.createWorld(scenario, { phase: 'acquisition' });
  worldAdapter.applyAction(world, { op: 'run', operation: 'pool_profile' });
  const bound = bindAcquisition(scenario, world);
  assert.deepEqual(bound.scenario.hUnits.map(u => [u.id, u.criterion]), scenario.hUnits.map(u => [u.id, u.criterion]));
  assert.equal(bound.acquisition.complete, false);
  assert.ok(bound.scenario.hUnits.some(u => u.sourceAvailability.some(s => !s.available)));
  assert.ok(!bound.sources.some(s => s.id === 'acq-dns_probe'));
});
test('withheld license control removes the original observation and all originating textual echoes', () => {
  const scenario = buildCorpus().find(s => s.id === 'v5-release-demanding-unresolved');
  const world = worldAdapter.createWorld(scenario, { phase: 'acquisition' });
  for (const a of worldAdapter.validAcquisitionPlan(scenario)) worldAdapter.applyAction(world, a);
  const bound = bindAcquisition(scenario, world, { summary: 'The license check passed.', traceSources: [{ id: 'origin-echo', actor: 'agent', conversation: 'origin', text: 'I saw the license check pass.' }] });
  const reduced = eligibleSources(bound.scenario, { withheld: true });
  for (const id of ['acq-validate_license', 'origin-summary', 'origin-echo']) assert.ok(!reduced.some(s => s.id === id));
  assert.ok(reduced.some(s => s.id === 'src-0011')); // Genuine unresolved identity remains present; it is not manufactured certainty.
});
