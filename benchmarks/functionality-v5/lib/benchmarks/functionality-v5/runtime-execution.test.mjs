import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, writeFile, rm, mkdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createRuntimeExecution, isolationCanaryPlan, PIN, APPROVED_SCREEN_BUDGET } from './runtime-execution.mjs';
import { hash } from './common.mjs';
import { accounting } from './accounting.mjs';

const conditions = hash({ corpus: 'fixture', protocol: 'v5-fixture' });
const good = (answer = '{}', tokens = 10) => ({ status: 'completed', answer, requested_model: PIN.model,
  returned_model: null, cli_version: PIN.cli, reasoning: PIN.reasoning, processing: PIN.processing,
  usage: { input_tokens: tokens - 2, output_tokens: 2, total_tokens: tokens, cached_input_tokens: 2, reasoning_output_tokens: 1 }, events: [], violations: [] });
const canaryResponse = prompt => {
  const nonce = /The calibration token is ([A-Za-z0-9_-]+)\./.exec(prompt)?.[1];
  return good(JSON.stringify(nonce ? { ack: nonce } : { token: null }));
};
async function fixture(t, options = {}) {
  const root = await mkdtemp(join(tmpdir(), 'loomtracer-v5-runtime-test-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  let clock = 100000, invoked = [];
  const defaults = { root, execute: true, evaluationConditionsHash: conditions, freeze: { corpus: hash('fixture') },
    minStartIntervalMs: 0, allowTestLimits: true, now: () => clock, wait: async ms => { clock += ms; },
    invoke: async args => { invoked.push(args); args.onEvent({ type: 'fixture.event' }); return canaryResponse(args.prompt); } };
  const runtime = await createRuntimeExecution({ ...defaults, ...options });
  return { root, runtime, defaults, invoked, clock: () => clock };
}
const task = (id = 'task-1') => ({ id, stage: 'reasoning', arm: 'stock-maintained-notes', caseId: 'case-fixture', trialId: id, prompt: 'Return JSON {}.' });

test('canary plan keeps the first-session nonce out of the second prompt', () => {
  const plan = isolationCanaryPlan();
  assert.ok(plan.first.prompt.includes(plan.nonce)); assert.ok(!plan.second.prompt.includes(plan.nonce));
  assert.throws(() => isolationCanaryPlan({ nonce: 'short' }), /Bounded/);
});

test('runtime import/setup never dispatch; explicit live approval and fresh canary required', async t => {
  const f = await fixture(t, { execute: false });
  assert.equal(f.invoked.length, 0);
  await assert.rejects(() => f.runtime.runInvocation(task()), /execution-disabled/);
  const g = await fixture(t);
  await assert.rejects(() => g.runtime.runInvocation(task()), /fresh-canary-required/);
  assert.equal(g.invoked.length, 0);
});

test('canaries, durable reservation and events precede each model call; all usage counted exactly once', async t => {
  const f = await fixture(t);
  const runtime = await createRuntimeExecution({ ...f.defaults, invoke: async args => {
    const directories = (await import('node:fs/promises')).readdir;
    const entries = await directories(join(f.root, 'calls'));
    const attempts = await Promise.all(entries.map(async id => JSON.parse(await readFile(join(f.root, 'calls', id, 'attempt.json'), 'utf8'))));
    assert.ok(attempts.some(attempt => attempt.promptHash === hash(args.prompt)));
    assert.equal(args.execute, true); assert.equal(args.timeoutMs, 120000); assert.equal(args.cwd, undefined);
    args.onEvent({ type: 'turn.completed', usage: good().usage }); return canaryResponse(args.prompt);
  } });
  const canary = await runtime.runCanaries({ batchId: 'pair' });
  assert.equal(canary.status, 'passed'); assert.equal(runtime.status().mayDispatch, true);
  const { call, value, parseError } = await runtime.runInvocation(task());
  assert.deepEqual(value, { token: null }); assert.equal(parseError, null);
  assert.equal(call.measurementScope, 'functionality-v5-cli-reported-invocations-1');
  assert.equal(call.requestedModel, PIN.model); assert.equal(call.cliVersion, PIN.cli);
  assert.equal(runtime.calls.length, 3); assert.equal(accounting(runtime.calls).reportedTokens, 30);
  const receipt = JSON.parse(await readFile(join(f.root, 'calls', 'task-1', 'receipt.json'), 'utf8'));
  assert.equal(Object.keys(receipt.files).length, 4);
  await runtime.verify();
  await assert.rejects(() => runtime.runInvocation(task()), /already-attempted/);
  await assert.rejects(() => runtime.runCanaries(), /already-attempted/);
});

test('failed invocation keeps usage and stops without retry', async t => {
  let n = 0;
  const f = await fixture(t, { invoke: async args => ++n <= 2 ? canaryResponse(args.prompt) : { ...good('partial', 18), status: 'failed', stop_reason: 'wall-time-limit' } });
  await f.runtime.runCanaries();
  const result = await f.runtime.runInvocation(task());
  assert.equal(result.call.status, 'failed'); assert.equal(f.runtime.status().usage.reportedTokens, 38);
  assert.equal(f.runtime.status().reason, 'invocation-failed');
  await assert.rejects(() => f.runtime.runInvocation(task('task-2')), /invocation-failed/); assert.equal(n, 3);
});

test('unknown usage preserves known subtotal and prevents subsequent dispatch', async t => {
  let n = 0;
  const f = await fixture(t, { invoke: async args => ++n <= 2 ? canaryResponse(args.prompt) : { ...good(), usage: null } });
  await f.runtime.runCanaries(); await f.runtime.runInvocation(task());
  assert.equal(f.runtime.status().usage.reportedTokens, 20); assert.equal(f.runtime.status().usage.unknownUsageCalls, 1);
  await assert.rejects(() => f.runtime.runInvocation(task('task-2')), /unknown-usage/); assert.equal(n, 3);
});

test('malformed participant JSON is saved without becoming an automatic retry or global stop', async t => {
  let n = 0;
  const f = await fixture(t, { invoke: async args => ++n <= 2 ? canaryResponse(args.prompt) : good('not JSON') });
  await f.runtime.runCanaries(); const result = await f.runtime.runInvocation(task());
  assert.equal(result.call.status, 'completed'); assert.equal(result.value, null); assert.ok(result.parseError);
  assert.equal(f.runtime.status().mayDispatch, true); await f.runtime.runInvocation(task('task-2')); assert.equal(n, 4);
});

test('budget includes canaries and stops the call after the threshold, without provider hard-cap claims', async t => {
  const f = await fixture(t, { budgetTokens: 25 });
  await f.runtime.runCanaries(); await f.runtime.runInvocation(task());
  assert.equal(f.runtime.status().usage.reportedTokens, 30); assert.equal(f.runtime.status().remainingKnownTokens, -5);
  await assert.rejects(() => f.runtime.runInvocation(task('task-2')), /threshold-reached/); assert.equal(f.invoked.length, 3);
});

test('approved ceiling counts acquisition, maintenance, tasks, controls, grading and canaries together', async t => {
  let n = 0;
  const f = await fixture(t, { budgetTokens: 4250000, invoke: async args => {
    n++;
    return n <= 2 ? canaryResponse(args.prompt) : good('{}', n === 10 ? 531244 : 531248);
  } });
  assert.equal(APPROVED_SCREEN_BUDGET, 4250000);
  await f.runtime.runCanaries();
  for (const stage of ['acquisition', 'maintenance', 'reasoning', 'action', 'retrieval', 'oracle', 'withheld', 'grading']) {
    await f.runtime.runInvocation({ ...task(`all-in-${stage}`), stage });
  }
  assert.equal(f.runtime.status().usage.reportedTokens, 4250000);
  assert.equal(f.runtime.status().remainingKnownTokens, 0);
  assert.equal(f.runtime.calls.length, 10);
  await assert.rejects(() => f.runtime.runInvocation(task('over-budget')), /threshold-reached/);
  assert.equal(n, 10);
});

test('leaked canary state blocks the experiment and preserves both calls', async t => {
  let n = 0;
  const f = await fixture(t, { invoke: async args => ++n === 1 ? canaryResponse(args.prompt) : good('{"token":"leak"}') });
  const result = await f.runtime.runCanaries(); assert.equal(result.status, 'failed'); assert.equal(f.runtime.calls.length, 2);
  await assert.rejects(() => f.runtime.runInvocation(task()), /isolation-canary-failed/);
  const resumed = await createRuntimeExecution(f.defaults);
  assert.equal(resumed.status().reason, 'isolation-canary-failed');
  await assert.rejects(() => resumed.runInvocation({ ...task('replacement'), stage: 'canary' }), /isolation-canary-failed/);
  assert.equal(n, 2);
});

test('completed but unsealed canary attempts cannot be replaced during continuation', async t => {
  const f = await fixture(t);
  await f.runtime.runInvocation({ ...task('unsealed-canary'), stage: 'canary' });
  const resumed = await createRuntimeExecution(f.defaults);
  assert.equal(resumed.status().reason, 'unsealed-canary-attempt');
  await assert.rejects(() => resumed.runInvocation({ ...task('replacement-canary'), stage: 'canary' }), /unsealed-canary-attempt/);
  assert.equal(f.invoked.length, 1);
});

test('unknown canary usage cannot establish a passing gate', async t => {
  const f = await fixture(t, { invoke: async args => ({ ...canaryResponse(args.prompt), usage: null }) });
  const result = await f.runtime.runCanaries(); assert.equal(result.status, 'failed'); assert.equal(f.runtime.calls.length, 1);
});

test('a second-canary runtime violation cannot be labeled passed even with correct text', async t => {
  let n = 0;
  const f = await fixture(t, { invoke: async args => ({ ...canaryResponse(args.prompt), violations: ++n === 2 ? ['unexpected-tool-or-item'] : [] }) });
  assert.equal((await f.runtime.runCanaries()).status, 'failed');
  assert.equal(f.runtime.status().reason, 'isolation-or-model-violation');
});

test('isolation and returned-model violations stop subsequent dispatch', async t => {
  for (const bad of [{ violations: ['unexpected-tool-or-item'] }, { returned_model: 'other-model' }]) {
    let n = 0;
    const f = await fixture(t, { invoke: async args => ++n <= 2 ? canaryResponse(args.prompt) : { ...good(), ...bad } });
    await f.runtime.runCanaries(); await f.runtime.runInvocation(task());
    assert.equal(f.runtime.status().reason, 'isolation-or-model-violation');
    await assert.rejects(() => f.runtime.runInvocation(task('task-2')), /isolation-or-model-violation/);
  }
});

test('snapshot verifier is checked before and after pacing and prevents changed-code dispatch', async t => {
  let verified = 0;
  const f = await fixture(t, { verifyBeforeDispatch: async () => { if (++verified === 2) throw new Error('frozen snapshot changed'); } });
  await assert.rejects(() => f.runtime.runInvocation({ ...task(), stage: 'canary' }), /frozen snapshot changed/);
  assert.equal(f.invoked.length, 0); assert.equal(f.runtime.calls.length, 0);
});

test('pacing is start-to-start and preserved across reopening', async t => {
  const f = await fixture(t, { minStartIntervalMs: 60000 });
  await f.runtime.runCanaries({ batchId: 'first' });
  assert.equal(f.runtime.calls[1].pacing.actual_start_ms - f.runtime.calls[0].pacing.actual_start_ms, 60000);
  const g = await createRuntimeExecution({ ...f.defaults, minStartIntervalMs: 60000 });
  assert.equal(g.status().reason, 'fresh-canary-required'); assert.equal(g.calls.length, 2);
  await g.runCanaries({ batchId: 'second' });
  assert.equal(g.calls[2].pacing.actual_start_ms - g.calls[1].pacing.actual_start_ms, 60000);
  await assert.rejects(() => g.runInvocation({ ...task('canary-first-1'), stage: 'canary' }), /already-attempted/);
});

test('variable preflight delays cannot compress observed CLI-start spacing', async t => {
  let clock = 100000;
  const starts = [], observations = [];
  const f = await fixture(t, { minStartIntervalMs: 60000, now: () => clock, wait: async ms => { clock += ms; },
    invoke: async args => {
      starts.push(clock);
      // The first CLI startup is much slower than the following one. Pacing
      // only from the adapter's entry would otherwise produce a 16-second gap.
      clock += starts.length === 1 ? 45000 : 1000;
      observations.push(clock); args.onEvent({ type: 'turn.started' });
      clock += 5000; return canaryResponse(args.prompt);
    } });
  await f.runtime.runCanaries({ batchId: 'variable-preflight' });
  assert.equal(starts[1], 205000);
  assert.equal(observations[1] - observations[0], 61000);
  assert.equal(f.runtime.calls[0].pacing.provider_start_observed_ms, observations[0]);
  assert.equal(f.runtime.calls[0].pacing.adapter_finished_ms, 150000);
  assert.equal(f.runtime.calls[0].pacing.provider_start_basis, 'turn.started-observation');
  const resumed = await createRuntimeExecution({ ...f.defaults, minStartIntervalMs: 60000,
    now: () => clock, wait: async ms => { clock += ms; }, invoke: async args => {
      starts.push(clock); clock += 1000; observations.push(clock); args.onEvent({ type: 'turn.started' });
      clock += 5000; return canaryResponse(args.prompt);
    } });
  await resumed.runCanaries({ batchId: 'after-reopen' });
  assert.ok(observations.slice(1).every((value, i) => value - observations[i] >= 60000));
});

test('unobserved CLI start uses adapter completion as a conservative pacing boundary', async t => {
  let clock = 100000;
  const starts = [];
  const f = await fixture(t, { minStartIntervalMs: 60000, now: () => clock, wait: async ms => { clock += ms; },
    invoke: async args => { starts.push(clock); clock += 20000; return canaryResponse(args.prompt); } });
  await f.runtime.runCanaries({ batchId: 'no-start-event' });
  assert.equal(starts[1] - starts[0], 80000);
  assert.equal(f.runtime.calls[0].pacing.provider_start_observed_ms, null);
  assert.equal(f.runtime.calls[0].pacing.provider_start_basis, 'adapter-completion-upper-bound');
  assert.ok(starts[1] >= f.runtime.calls[0].pacing.adapter_finished_ms + 60000);
});

test('immutable result, prompt, event and manifest changes are detected', async t => {
  for (const file of ['prompt.txt', 'result.json', 'events.jsonl', 'attempt.json']) {
    const f = await fixture(t); await f.runtime.runCanaries({ batchId: 'pair' });
    await writeFile(join(f.root, 'calls', 'canary-pair-1', file), 'changed');
    await assert.rejects(() => f.runtime.verify(), /integrity/);
  }
  const f = await fixture(t); await writeFile(join(f.root, 'runtime-manifest.json'), '{}');
  await assert.rejects(() => f.runtime.verify(), /manifest file changed/);
  await assert.rejects(() => createRuntimeExecution({ ...f.defaults, freeze: { corpus: 'changed' } }), /manifest changed/);
});

test('orphaned attempt is unknown expenditure and is never redispatched', async t => {
  const f = await fixture(t); const directory = join(f.root, 'calls', 'orphan');
  await mkdir(directory); await writeFile(join(directory, 'attempt.json'), JSON.stringify({ metadata: { stage: 'action' }, promptHash: hash('partial') }));
  const resumed = await createRuntimeExecution(f.defaults);
  assert.equal(resumed.calls.length, 1); assert.equal(resumed.calls[0].status, 'interrupted');
  assert.equal(resumed.status().reason, 'unresolved-attempt'); assert.equal(resumed.status().usage.unknownUsageCalls, 1);
  await assert.rejects(() => resumed.runInvocation({ ...task('orphan'), stage: 'canary' }), /already-attempted/);
  assert.equal(f.invoked.length, 0);
});

test('a missing invocation directory cannot disappear from expenditure during continuation', async t => {
  const f = await fixture(t); await f.runtime.runCanaries({ batchId: 'pair' });
  await rm(join(f.root, 'calls', 'canary-pair-1'), { recursive: true });
  await assert.rejects(() => f.runtime.verify(), /invocation disappeared/);
  await assert.rejects(() => createRuntimeExecution(f.defaults), /invocation disappeared/);
});

test('failed save boundary and external dispatch lock prevent another provider call', async t => {
  const f = await fixture(t); await writeFile(join(f.root, 'dispatch.lock'), 'existing');
  await assert.rejects(() => f.runtime.runInvocation({ ...task(), stage: 'canary' }), /existing-dispatch-lock/);
  assert.equal(f.invoked.length, 0);
});

test('live settings cannot expand the budget, disable pacing or replace the provider timeout', async t => {
  const f = await fixture(t);
  await assert.rejects(() => createRuntimeExecution({ ...f.defaults, budgetTokens: APPROVED_SCREEN_BUDGET + 1 }), /ceiling/);
  await assert.rejects(() => createRuntimeExecution({ ...f.defaults, allowTestLimits: false, minStartIntervalMs: 0 }), /60-second/);
  await assert.rejects(() => createRuntimeExecution({ ...f.defaults, timeoutMs: 120001 }), /120 seconds/);
  await assert.rejects(() => createRuntimeExecution({ ...f.defaults, invoke: undefined }), /checkpoint verifier/);
  await assert.rejects(() => createRuntimeExecution({ ...f.defaults, invoke: undefined, verifyBeforeDispatch: async () => {} }), /mock invocation/);
});

test('final summary is immutable and includes invocation receipt hashes', async t => {
  const f = await fixture(t); await f.runtime.runCanaries({ batchId: 'pair' });
  const summary = await f.runtime.seal(); assert.equal(Object.keys(summary.invocationReceiptHashes).length, 2);
  await assert.rejects(() => f.runtime.seal(), /EEXIST/);
});
