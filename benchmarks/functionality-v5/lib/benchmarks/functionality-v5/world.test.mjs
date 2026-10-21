import test from 'node:test';
import assert from 'node:assert/strict';
import { buildCorpus, selectedScreenIds, bindAcquisition } from './corpus.mjs';
import { createWorld, applyAction, applyBatch, validAcquisitionPlan, validActionPlan, alternateActionPlan, deterministicChecks, publicWorldContract } from './world.mjs';
import { performWorkflow, taskPrompt } from './workflow.mjs';
import { controlSources } from './run-screen.mjs';

const corpus = buildCorpus();
function acquire(s) {
  const a = createWorld(s, { phase: 'acquisition' });
  for (const action of validAcquisitionPlan(s)) applyAction(a, action);
  assert.equal(a.status, 'complete');
  return a;
}
function actionWorld(s, options = {}) {
  const a = acquire(s);
  return createWorld(s, { acquisition: a, eligibleSourceIds: ['src-0011'], visibleArtifactIds: Object.keys(a.artifacts), ...options });
}
function execute(w, plan) {
  for (const action of plan) if (action.op !== 'lookup') applyAction(w, action);
  return deterministicChecks(w._scenario, w);
}
const selected = selectedScreenIds.map(id => corpus.find(x => x.id === id));

test('all 27 complete acquisition workflows produce actual receipts including original release manifest', () => {
  for (const s of corpus) {
    const a = acquire(s);
    assert.ok(a.receipts.every(x => x.ok));
    assert.ok(Object.values(a.artifacts).every(x => x.hash && x.receiptId));
    if (s.family === 'release') assert.ok(a.artifacts['original-manifest']);
  }
});
test('all 27 current plans fit twelve operations including lookup and pass strict local checks', () => {
  for (const s of corpus) {
    const plan = validActionPlan(s);
    assert.ok(plan.length <= 12, `${s.id}: ${plan.length}`);
    assert.equal(plan.filter(x => x.op === 'lookup').length, 1);
    const w = actionWorld(s);
    const checks = execute(w, plan);
    assert.equal(checks.complete, true, s.id);
    assert.ok([...checks.Q, ...checks.W].every(x => x.status === 'pass'), s.id);
    assert.equal(checks.correctlyBlocked, s.identity === 'unresolved');
    assert.equal(checks.resolvedWork, s.identity !== 'unresolved');
  }
});
test('all alternate independent reuse orders are accepted without a golden transcript', () => {
  for (const s of corpus) assert.equal(execute(actionWorld(s), alternateActionPlan(s)).complete, true, s.id);
});
test('all withheld witnesses preserve unknown historical evidence and safely complete or block', () => {
  for (const s of corpus) {
    const w = actionWorld(s, { withheld: true });
    const p = validActionPlan(s, { withheld: true });
    assert.ok(p.length <= 12);
    assert.equal(execute(w, p).correctlyBlocked, true, s.id);
  }
});
test('no-memory guessed artifact names and undelivered mapping cannot complete a workflow', () => {
  const s = selected[0];
  const w = actionWorld(s, { eligibleSourceIds: [], visibleArtifactIds: [] });
  const reuse = applyAction(w, { op: 'reuse', artifactId: 'pool-profile' });
  assert.equal(reuse.error.code, 'unretained_artifact');
  const run = applyAction(w, { op: 'run', operation: 'configure_trial', inputs: { pool_cap: 12, environment: 'staging' } });
  assert.equal(run.error.code, 'mapping_not_delivered');
  assert.equal(deterministicChecks(s, w).complete, false);
});
test('public tools contain no historical answers, hidden identity key or permitted source archive', () => {
  for (const s of selected) {
    const contract = JSON.stringify(publicWorldContract(s));
    assert.ok(!contract.includes('worldDefinition'));
    assert.ok(!contract.includes('pool_cap":12'));
    assert.ok(!contract.includes('classification":"' + s.identity + '"'));
    assert.ok(!contract.includes(s.id));
  }
});
test('necessary conditional questions cannot be grouped with their unanswered discriminator', () => {
  const w = actionWorld(selected[0]);
  const result = applyAction(w, { op: 'ask', fields: ['pool_cap', 'alert_recipient'] });
  assert.equal(result.answers.pool_cap.status, 'answered');
  assert.equal(result.answers.alert_recipient.status, 'premature');
  assert.ok(!Object.hasOwn(w.questionAnswers, 'alert_recipient'));
});
test('a premature separately scored necessary question fails once without an extraneous-question penalty', () => {
  const s = selected[0], w = actionWorld(s);
  applyAction(w, { op: 'ask', fields: ['alert_recipient'] });
  const q = execute(w, validActionPlan(s)).Q;
  assert.equal(q.find(x => x.id === 'q-necessary-alert_recipient').status, 'fail');
  assert.equal(q.find(x => x.id === 'q-no-extraneous-questions').status, 'pass');
  assert.equal(q.filter(x => x.status === 'fail').length, 1);
  assert.equal(q.length, s.qUnits.length, 'The strict obligations and denominator are unchanged');
});
test('a separately scored already-known question fails only its own avoid obligation', () => {
  const s = selected[0], w = actionWorld(s);
  applyAction(w, { op: 'ask', fields: ['environment'] });
  const q = execute(w, validActionPlan(s)).Q;
  assert.equal(q.find(x => x.id === 'q-avoid-environment').status, 'fail');
  assert.equal(q.find(x => x.id === 'q-no-extraneous-questions').status, 'pass');
  assert.equal(q.filter(x => x.status === 'fail').length, 1);
});
test('an extra question outside all separately scored fields still fails the guard', () => {
  const s = selected[0], w = actionWorld(s);
  applyAction(w, { op: 'ask', fields: ['favorite_color'] });
  const q = execute(w, validActionPlan(s)).Q;
  assert.equal(q.find(x => x.id === 'q-no-extraneous-questions').status, 'fail');
  assert.equal(q.filter(x => x.status === 'fail').length, 1);
});
test('known, inactive, repeated and unlisted questions fail relevant Q obligations', () => {
  const s = selected[0];
  const w = actionWorld(s);
  applyAction(w, { op: 'ask', fields: ['environment', 'production_region', 'favorite_color'] });
  execute(w, validActionPlan(s));
  const q = deterministicChecks(s, w).Q;
  assert.equal(q.find(x => x.id === 'q-avoid-environment').status, 'fail');
  assert.equal(q.find(x => x.id === 'q-avoid-production_region').status, 'fail');
  assert.equal(q.find(x => x.id === 'q-no-extraneous-questions').status, 'fail');
});
test('doing nothing does not pass avoid-question obligations that require historical input use', () => {
  const s = selected[0];
  const checks = deterministicChecks(s, actionWorld(s));
  assert.equal(checks.Q.find(x => x.id === 'q-avoid-environment').status, 'fail');
  assert.ok(checks.Q.filter(x => x.kind === 'necessary').every(x => x.status === 'fail'));
});
test('known inputs accept equivalent supplied forms, not only one exact string representation', () => {
  const s = selected[0];
  const w = actionWorld(s);
  const plan = validActionPlan(s).map(a => a.operation === 'run_current_canary' ? { ...a, inputs: { ...a.inputs, staffing: 'two-person staffing' } } : a);
  assert.equal(execute(w, plan).complete, true);
});
function verifyShift(value) {
  const s = selected[1], world = actionWorld(s);
  applyAction(world, { op: 'ask', fields: ['buffer'] });
  applyAction(world, { op: 'ask', fields: ['operator'] });
  applyAction(world, { op: 'reuse', artifactId: 'safety-check' });
  const action = { op: 'run', operation: 'verify_operator', inputs: { operator: s.worldDefinition.unknownFields.operator, shift: value } };
  return { scenario: s, world, action, receipt: applyAction(world, action) };
}
test('complete equivalent shift expressions preserve raw input and produce the same verified meaning', () => {
  const variants = [
    'one technician, daytime', 'daytime, one technician', 'one-technician daytime shift',
    'daytime shift; one-technician', 'on-site daytime shift, one technician',
    'one technician, on-site, daytime', 'daytime one technician shift on-site',
    '1 technician; daytime', 'DAYTIME; 1-TECHNICIAN SHIFT', 'one‑technician daytime shift',
  ];
  let observation;
  for (const value of variants) {
    const { world, action, receipt } = verifyShift(value);
    assert.equal(receipt.ok, true, value);
    assert.equal(receipt.artifact.inputs.shift, value, 'Raw submitted input must not be overwritten by normalization');
    assert.deepEqual(world.actions.at(-1).action, action, 'The exact submitted operation remains in the trace');
    observation ??= receipt.artifact.observation;
    assert.equal(receipt.artifact.observation, observation);
    assert.match(observation, /one-technician daytime controlled batch/);
  }
});
test('legacy frozen scenarios receive the complete public shift contract without source or rubric mutation', () => {
  const legacy = structuredClone(selected[1]);
  const oldOperation = legacy.worldDefinition.fresh.find(x => x.operation === 'verify_operator');
  delete oldOperation.publicDescription;
  legacy.contract.workflow = 'verify_operator: Verify the supplied operator initials against the one-technician shift and safe enclosure receipt.';
  legacy.episodes[2].sources.find(x => x.id === 'src-0012').text = legacy.contract.workflow;
  const before = structuredClone(legacy);
  const contract = publicWorldContract(legacy);
  const description = contract.operations.find(x => x.operation === 'verify_operator').description;
  assert.match(description, /both staffing and time of day together/);
  assert.match(description, /values established in retained sources/);
  assert.doesNotMatch(description, /one technician|one-technician|daytime|nighttime/);
  assert.equal(description, publicWorldContract(selected[1]).operations.find(x => x.operation === 'verify_operator').description);
  const prompt = taskPrompt({ scenario: legacy, task: { kind: 'action', mode: 'ordinary', control: 'oracle' },
    history: [], trace: [], operationsUsed: 12 });
  assert.ok(prompt.includes(description));
  assert.ok(!prompt.includes(legacy.worldDefinition.knownFields.shift));
  assert.deepEqual(legacy, before, 'A public projection must preserve original source, criterion and private-world data');
});
test('incomplete, contradictory, ambiguous or qualified shifts fail closed', () => {
  const invalid = [
    'one-technician', 'one technician shift', 'daytime', 'daytime shift',
    'two technicians, daytime', '2 technicians, daytime', 'one technician, night',
    'one technician, nighttime', 'one technician, unknown', 'unknown, daytime',
    'one technician, daytime or night', 'one or two technicians, daytime',
    'not one technician, daytime', 'one technician, not daytime',
    'one technician, daytime except Friday', 'one technician, usually daytime',
    'at least one technician, daytime', 'one technician, daytime only for emergencies',
    'one technician, daytime, remote', 'one technician, daytime, off-site',
    'one technician, daytime, no supervision', 'one technician, daytime, nighttime',
    'one technician one technician daytime', 'one technician daytime daytime',
    'one technician, daytime shift shift', 'one technician, daytime on-site on-site',
    '', null, 1, { staffing: 1, timeOfDay: 'daytime' },
  ];
  for (const value of invalid) {
    const { scenario, world, receipt } = verifyShift(value);
    assert.equal(receipt.ok, false, JSON.stringify(value));
    assert.equal(receipt.error.code, 'missing_or_wrong_input');
    assert.ok(!world.artifacts['operator-verification']);
    assert.equal(deterministicChecks(scenario, world).W.find(x => x.id === 'w-fresh-operator-verification').status, 'fail');
  }
});
test('complete five-call experiment witness accepts reordered shift within unchanged operation and delivery limits', async t => {
  const original = selected[1], acquisition = acquire(original);
  const scenario = bindAcquisition(original, acquisition).scenario;
  const task = { id: 'offline-shift-equivalence', kind: 'action', mode: 'ordinary', control: 'oracle' };
  const sources = controlSources(scenario, task);
  const plan = validActionPlan(scenario).filter(x => x.op !== 'lookup');
  const run = operation => structuredClone(plan.find(x => x.operation === operation));
  const verifyOperator = run('verify_operator');
  verifyOperator.inputs.shift = 'daytime shift; on-site one-technician';
  const batches = [
    [...plan.filter(x => x.op === 'ask' && x.fields.includes('buffer')), ...plan.filter(x => x.op === 'reuse')],
    [...plan.filter(x => x.op === 'ask' && x.fields.includes('operator')), run('prepare_batch')],
    [run('analyze_batch'), verifyOperator],
    [run('package_analysis')],
    [plan.at(-1)],
  ];
  let calls = 0;
  const result = await performWorkflow({ scenario, task, acquisition, sources, save: async () => {}, obtain: async () => {
    assert.ok(calls < batches.length, 'The complete valid witness needs exactly five mocked calls');
    const value = { actions: batches[calls++] };
    return { value, parseError: null, call: { status: 'completed', usage: { input_tokens: 1, output_tokens: 1, total_tokens: 2 }, answer: JSON.stringify(value) } };
  } });
  assert.equal(calls, 5);
  assert.equal(result.status, 'completed');
  assert.deepEqual(result.world.errors, []);
  assert.ok(result.operationsUsed <= 16);
  assert.ok(result.view.historicalBytes <= 32768);
  assert.equal(result.world.artifacts['operator-verification'].inputs.shift, verifyOperator.inputs.shift);
  assert.ok([...deterministicChecks(scenario, result.world).Q, ...deterministicChecks(scenario, result.world).W].every(x => x.status === 'pass'));
  t.diagnostic(`Five mocked calls, ${result.operationsUsed} operations, ${result.view.historicalBytes} cumulative historical UTF-8 bytes; zero provider calls.`);
});
test('an unnecessary rerun after recognition passes timing but fails work reuse', () => {
  const s = selected[0];
  const w = actionWorld(s);
  applyAction(w, { op: 'recognize', classification: 'same', evidence: ['src-0011'], explanation: 'Registry maps this request to the original OPS-184 failure episode.' });
  applyAction(w, { op: 'run', operation: 'pool_profile' });
  const checks = execute(w, validActionPlan(s));
  assert.equal(checks.D.late, false);
  assert.equal(checks.W.find(x => x.id === 'w-reuse-pool-profile').status, 'fail');
  assert.equal(checks.W.find(x => x.id === 'w-no-extraneous-work').status, 'fail');
});
test('stale old results cannot substitute for current execution', () => {
  const w = actionWorld(selected[0]);
  assert.equal(applyAction(w, { op: 'reuse', artifactId: 'old-canary' }).error.code, 'stale_or_inapplicable');
  assert.equal(applyAction(w, { op: 'run', operation: 'old_canary' }).error.code, 'stale_operation');
});
test('skipped necessary work fails its unit without erasing independent successful milestones', () => {
  const s = selected[0];
  const w = actionWorld(s);
  const checks = execute(w, validActionPlan(s).filter(x => x.operation !== 'run_current_canary'));
  assert.equal(checks.W.find(x => x.id === 'w-fresh-current-config').status, 'pass');
  assert.equal(checks.W.find(x => x.id === 'w-fresh-current-canary').status, 'fail');
  assert.equal(checks.complete, false);
});
test('late and unsupported identity declarations are retained, never validated by the broker', () => {
  const s = selected[1];
  const w = actionWorld(s);
  applyAction(w, { op: 'reuse', artifactId: 'calibration' });
  const r = applyAction(w, { op: 'recognize', classification: 'same', evidence: ['made-up-source'], explanation: 'Numbers match.' });
  assert.equal(r.ok, true);
  assert.equal(r.assessmentRecorded, true);
  assert.equal(deterministicChecks(s, w).D.late, true);
  assert.equal(deterministicChecks(s, w).D.semanticStatus, 'pending');
});
test('dependent run, question and finish operations must await the next invocation receipt', () => {
  const a = createWorld(selected[0], { phase: 'acquisition' });
  const receipts = applyBatch(a, [
    { op: 'run', operation: 'pool_profile' }, { op: 'run', operation: 'dns_probe' }, { op: 'run', operation: 'old_canary' },
  ]);
  assert.equal(receipts[0].ok, true); assert.equal(receipts[1].ok, true);
  assert.equal(receipts[2].error.code, 'dependent_batch');
  const w = actionWorld(selected[0]);
  const q = applyBatch(w, [{ op: 'ask', fields: ['pool_cap'] }, { op: 'ask', fields: ['alert_recipient'] }]);
  assert.equal(q[1].error.code, 'dependent_batch');
  const b = createWorld(selected[0], { phase: 'acquisition' });
  const start = structuredClone(b);
  applyAction(b, { op: 'run', operation: 'pool_profile' }, { batchStart: start });
  const done = applyAction(b, { op: 'finish', summary: 'Pretend complete', artifacts: ['pool-profile'] }, { batchStart: start });
  assert.equal(done.error.code, 'dependent_batch');
});
test('correctly blocked unresolved case completes safe work without creating release authority', () => {
  const s = selected[2];
  const w = actionWorld(s);
  const checks = execute(w, validActionPlan(s));
  assert.equal(checks.correctlyBlocked, true);
  assert.equal(checks.complete, true);
  assert.equal(checks.resolvedWork, false);
  assert.ok(w.artifacts['validation-signoff']);
  assert.ok(!w.artifacts['release-manifest']);
});
test('withheld historical license result can be rechecked now without fabricating its past', () => {
  const s = selected[2];
  const w = actionWorld(s, { withheld: true });
  const checks = execute(w, validActionPlan(s, { withheld: true }));
  assert.equal(checks.correctlyBlocked, true);
  assert.ok(w.artifacts['repeated-license-check']);
  assert.ok(w.artifacts['validation-signoff']);
  assert.ok(!w.reused.some(x => x.artifactId === 'license-check'));
});
test('missing acquisition work remains missing; shared-world copies isolate successor forks', () => {
  const s = selected[0];
  const a = createWorld(s, { phase: 'acquisition' });
  applyAction(a, { op: 'run', operation: 'pool_profile' });
  const w = createWorld(s, { acquisition: a, visibleArtifactIds: ['dns-trace'], eligibleSourceIds: ['src-0011'] });
  assert.equal(applyAction(w, { op: 'reuse', artifactId: 'dns-trace' }).error.code, 'missing_actual_artifact');
  const origin = acquire(s);
  const forkA = createWorld(s, { acquisition: origin });
  const forkB = createWorld(s, { acquisition: origin });
  forkA.priorArtifacts['pool-profile'].observation = 'Changed in one fork';
  assert.notEqual(forkA.priorArtifacts['pool-profile'].observation, forkB.priorArtifacts['pool-profile'].observation);
});
test('artifact and receipt reference output forms work; invented and unexecuted refs fail', () => {
  const s = selected[0];
  for (const form of ['artifact', 'object', 'receipt']) {
    const w = actionWorld(s);
    const plan = validActionPlan(s);
    execute(w, plan.slice(0, -1));
    const final = structuredClone(plan.at(-1));
    final.artifacts = Object.values(w.artifacts).map(x => form === 'object' ? { artifactId: x.id } : form === 'receipt' ? x.receiptId : x.id);
    assert.equal(applyAction(w, final).ok, true);
  }
  const w = actionWorld(s);
  execute(w, validActionPlan(s).slice(0, -1));
  assert.equal(applyAction(w, { op: 'finish', summary: 'Done', artifacts: ['never-executed'] }).error.code, 'invalid_final_reference');
});
test('future operations are absent during acquisition and operation count is bounded', () => {
  const w = createWorld(selected[0], { phase: 'acquisition' });
  assert.equal(applyAction(w, { op: 'run', operation: 'configure_trial' }).error.code, 'future_operation');
  for (let i = 0; i < 16; i += 1) applyAction(w, { op: 'message', text: 'A recorded note.' });
  assert.equal(w.receipts.at(-1).error.code, 'operation_limit');
});
