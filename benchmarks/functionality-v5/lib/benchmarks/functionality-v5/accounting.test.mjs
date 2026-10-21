import test from 'node:test';
import assert from 'node:assert/strict';
import { accounting, usageReport, normalizedUsage, tokenDifference, dispatchStatus, compareOverhead, checkpointUsage, MEASUREMENT_SCOPE } from './accounting.mjs';

const call = (id, stage, overrides = {}) => ({ id, stage, status: 'completed', usage: { input_tokens: 100, output_tokens: 20, cached_input_tokens: 40, reasoning_output_tokens: 5 },
  arm: 'notes', caseId: 'case', episode: 1, checkpointGroup: 'checkpoint', elapsed_ms: 20,
  evaluationConditionsHash: 'same-conditions', requestedModel: 'same-model', returnedModel: null, reasoning: 'medium', cliVersion: 'same-cli', measurementScope: MEASUREMENT_SCOPE, ...overrides });

test('cached and reasoning tokens are subsets and uncached requires known cached input', () => {
  assert.deepEqual(normalizedUsage(call('x', 'action').usage), { input: 100, output: 20, total: 120, cachedInput: 40, reasoningOutput: 5, uncachedInput: 60 });
  assert.equal(normalizedUsage({ input_tokens: 10, output_tokens: 2 }).uncachedInput, null);
  assert.equal(normalizedUsage({ input_tokens: 10, output_tokens: 2, cached_input_tokens: 11 }).cachedInput, null);
  assert.equal(normalizedUsage({ input_tokens: 10, output_tokens: 2, reasoning_tokens: 3 }).reasoningOutput, null);
  assert.equal(normalizedUsage({ input_tokens: 10, output_tokens: 2, total_tokens: 50 }).total, null);
});
test('failed calls and known subtotals survive unknown usage and stop dispatch', () => {
  const calls = [call('one', 'action', { status: 'failed' }), call('two', 'grading', { usage: null })];
  const result = accounting(calls);
  assert.equal(result.reportedTokens, 120); assert.equal(result.completeTotalTokens, null);
  assert.equal(result.unknownUsageCalls, 1); assert.equal(result.failedCalls, 1);
  assert.equal(dispatchStatus(calls, 4250000).reason, 'unknown-usage');
  assert.equal(dispatchStatus([calls[0]], 120).reason, 'threshold-reached');
});
test('duplicate IDs are rejected, not deduplicated invisibly', () => {
  assert.throws(() => accounting([call('same', 'action'), call('same', 'maintenance')]), /Duplicate/);
  assert.throws(() => accounting([call('x', 'unclassified')]), /stage/);
});
test('mock complete three-case pilot counts every invocation once and excludes shared costs', () => {
  const calls = [call('canary-1', 'canary'), call('canary-2', 'canary')];
  for (let i = 0; i < 3; i++) {
    const scenarioId = `case-${i}`;
    calls.push(call(`origin-${i}`, 'acquisition', { scenarioId }));
    for (let episode = 1; episode <= 4; episode++) calls.push(call(`maint-${i}-${episode}`, 'maintenance', { scenarioId, episode }));
    calls.push(call(`H-${i}`, 'reasoning', { scenarioId }));
    for (const mode of ['ordinary', 'prompted']) calls.push(call(`action-${i}-${mode}`, 'action', { scenarioId, mode, mixedActivities: ['lookup', 'question', 'work'] }));
    for (const stage of ['oracle', 'withheld', 'grading']) calls.push(call(`${stage}-${i}`, stage, { scenarioId }));
  }
  const r = usageReport(calls);
  assert.equal(r.total.calls, 35); assert.equal(r.total.reportedTokens, 35 * 120);
  assert.equal(r.primary.calls, 21); assert.equal(r.arms.notes.calls, 21);
  assert.equal(r.acquisition.calls, 3); assert.equal(r.shared.calls, 14);
  assert.equal(r.stages.action.calls, 6); assert.equal(r.stages.retrieval.calls, 0);
  assert.equal(Object.values(r.stages).reduce((n, stage) => n + stage.calls, 0), calls.length);
  assert.equal(r.primary.calls + r.shared.calls, calls.length);
});
test('H alias is a single reasoning stage; checkpoint totals are explicitly non-additive', () => {
  const calls = [call('m1', 'maintenance', { scenarioId: 'case', episode: 1 }), call('m2', 'maintenance', { scenarioId: 'case', episode: 2 }), call('h', 'H', { scenarioId: 'case', trialId: 'h' })];
  assert.equal(usageReport(calls).stages.reasoning.calls, 1);
  const checkpoint = checkpointUsage(calls, { arm: 'notes', scenarioId: 'case', captureEpisode: 1, trialIds: ['h'] });
  assert.equal(checkpoint.maintenanceToDate.calls, 1); assert.equal(checkpoint.currentExecution.calls, 1); assert.equal(checkpoint.nonAdditive, true);
});
test('zero denominator retains absolute difference and no ratio', () => {
  assert.deepEqual(tokenDifference(50, 0), { status: 'available', reason: 'zero-baseline-denominator', difference: 50, ratio: null, percentageDifference: null });
  assert.equal(tokenDifference(50, 100).percentageDifference, -50);
  assert.equal(tokenDifference(null, 100).status, 'unavailable');
});
function paired() {
  const planned = [], calls = [], results = [];
  for (const arm of ['notes', 'loom']) {
    const checkpointGroup = `${arm}-checkpoint`;
    calls.push(call(`${arm}-maintenance`, 'maintenance', { arm, checkpointGroup }));
    for (const kind of ['H', 'action']) {
      const id = `${arm}-${kind}`;
      planned.push({ id, arm, caseId: 'case', checkpointGroup, kind, mode: 'ordinary', repetition: 1, maintenanceRequired: true });
      calls.push(call(id, kind === 'H' ? 'reasoning' : 'action', { arm, checkpointGroup, trialId: id }));
      results.push({ id, status: 'completed', quality: 'pending' });
    }
  }
  calls.push(call('shared', 'grading', { usage: { total_tokens: 999999 } }));
  return { calls, planned, results, candidate: 'loom', baseline: 'notes' };
}
test('paired overhead counts shared maintenance once and excludes grading', () => {
  const r = compareOverhead(paired());
  assert.equal(r.status, 'available'); assert.equal(r.candidate.calls, 3); assert.equal(r.candidate.reportedTokens, 360);
  assert.equal(r.differences.ratio, 1); assert.equal(r.exploratory, true);
});
test('incomplete, failed, unknown and incompatible comparisons cannot look cheaper', () => {
  for (const mutation of [
    p => { p.results[0].status = 'failed'; },
    p => { p.calls[0].status = 'failed'; },
    p => { p.calls[0].usage = null; },
    p => { p.calls[0].measurementScope = 'old-version'; },
    p => { p.planned[0].caseId = 'other-case'; }
  ]) { const p = paired(); mutation(p); const r = compareOverhead(p); assert.equal(r.status, 'unavailable'); assert.equal(r.differences, null); }
});
