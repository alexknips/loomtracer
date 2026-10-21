/** Counts saved invocations, including failed calls. Subsets are never added to totals. */
export const MEASUREMENT_SCOPE = 'functionality-cli-reported-invocations-v1';
export const TOKEN_FIELDS = ['input', 'output', 'total', 'cachedInput', 'uncachedInput', 'reasoningOutput'];
const count = value => Number.isSafeInteger(value) && value >= 0 ? value : null;
const idOf = arm => typeof arm === 'string' ? arm : arm.id;
const shared = call => call.stage === 'canary' || call.stage === 'grading';

export function normalizedUsage(usage) {
  const input = count(usage?.input_tokens), output = count(usage?.output_tokens);
  let total = count(usage?.total_tokens);
  if (input !== null && output !== null) {
    const sum = count(input + output);
    total = total === null ? sum : total === sum ? total : null;
  }
  let cachedInput = count(usage?.cached_input_tokens ?? usage?.input_tokens_details?.cached_tokens);
  let reasoningOutput = count(usage?.reasoning_output_tokens ?? usage?.reasoning_tokens ?? usage?.output_tokens_details?.reasoning_tokens);
  if (cachedInput !== null && (input === null || cachedInput > input)) cachedInput = null;
  if (reasoningOutput !== null && (output === null || reasoningOutput > output)) reasoningOutput = null;
  return { input, output, total, cachedInput, reasoningOutput,
    uncachedInput: input !== null && cachedInput !== null ? input - cachedInput : null };
}

function validateLedger(calls) {
  const seen = new Set();
  for (const call of calls) {
    if (typeof call.id !== 'string' || !call.id) throw new Error('Every invocation needs a stable id');
    if (seen.has(call.id)) throw new Error(`Duplicate invocation id: ${call.id}`);
    seen.add(call.id);
  }
}

export function accounting(calls = []) {
  validateLedger(calls);
  const values = calls.map(call => normalizedUsage(call.usage));
  const tokens = Object.fromEntries(TOKEN_FIELDS.map(field => {
    const known = values.map(value => value[field]).filter(value => value !== null);
    const knownSubtotal = known.reduce((sum, value) => sum + value, 0);
    if (!Number.isSafeInteger(knownSubtotal)) throw new Error(`Token subtotal exceeds safe integer: ${field}`);
    const unknownCalls = calls.length - known.length;
    return [field, { knownSubtotal, completeTotal: unknownCalls ? null : knownSubtotal,
      knownCalls: known.length, unknownCalls }];
  }));
  const timings = calls.map(call => count(call.elapsed_ms));
  return { measurementScope: MEASUREMENT_SCOPE, calls: calls.length,
    callIds: calls.map(call => call.id), tokens,
    knownUsageCalls: tokens.total.knownCalls, unknownUsageCalls: tokens.total.unknownCalls,
    reportedTokens: tokens.total.knownSubtotal, completeTotalTokens: tokens.total.completeTotal,
    failedCalls: calls.filter(call => call.status !== 'completed').length,
    elapsedMs: timings.filter(value => value !== null).reduce((sum, value) => sum + value, 0),
    unknownTimingCalls: timings.filter(value => value === null).length };
}
export const aggregateUsage = accounting;

export function usageBreakdown({ calls = [], arms = [], scenarios = [] }) {
  validateLedger(calls);
  const stages = rows => Object.fromEntries(['maintenance', 'reasoning', 'action'].map(stage =>
    [stage, accounting(rows.filter(call => call.stage === stage))]));
  const experiment = calls.filter(call => !shared(call));
  return { measurementScope: MEASUREMENT_SCOPE, total: accounting(calls),
    experiment: accounting(experiment),
    shared: { total: accounting(calls.filter(shared)), canary: accounting(calls.filter(call => call.stage === 'canary')),
      grading: accounting(calls.filter(call => call.stage === 'grading')) },
    unassigned: accounting(experiment.filter(call => !arms.some(arm => idOf(arm) === call.arm))),
    arms: arms.map(arm => {
      const rows = experiment.filter(call => call.arm === idOf(arm));
      return { arm: idOf(arm), label: arm.label ?? idOf(arm), total: accounting(rows), stages: stages(rows),
        scenarios: scenarios.map(scenario => {
          const selected = rows.filter(call => call.scenarioId === scenario.id);
          return { scenarioId: scenario.id, total: accounting(selected), stages: stages(selected) };
        }) };
    }) };
}

export function tokenDifference(candidateTokens, baselineTokens) {
  if (count(candidateTokens) === null || count(baselineTokens) === null) return {
    status: 'unavailable', reason: 'unknown-usage', difference: null, ratio: null, percentageDifference: null };
  return { status: 'available', reason: baselineTokens === 0 ? 'zero-baseline-denominator' : null,
    difference: candidateTokens - baselineTokens,
    ratio: baselineTokens === 0 ? null : candidateTokens / baselineTokens,
    percentageDifference: baselineTokens === 0 ? null : 100 * (candidateTokens - baselineTokens) / baselineTokens };
}

function kind(arm) { return typeof arm === 'string' ? arm : arm.kind ?? arm.id; }
const isTranscript = arm => /transcript/.test(kind(arm));
const isStock = arm => /note|markdown|transcript/.test(kind(arm)) && !/loom/.test(kind(arm));

function scenarioScope({ calls, results, arm, scenario }) {
  const armId = idOf(arm), rows = calls.filter(call => !shared(call) && call.arm === armId && call.scenarioId === scenario.id);
  const resultRows = results.filter(result => result.arm === armId && result.scenarioId === scenario.id);
  const errors = [];
  if (resultRows.length !== 1 || resultRows[0].status !== 'completed') errors.push('scenario-not-completed');
  for (const stage of ['reasoning', 'action']) {
    if (!rows.some(call => call.stage === stage)) errors.push(`missing-${stage}-invocation`);
  }
  if (!isTranscript(arm) && resultRows[0]?.maintenance?.status !== 'not_required' && !rows.some(call => call.stage === 'maintenance')) {
    errors.push('missing-maintenance-invocation');
  }
  if (rows.some(call => call.status !== 'completed')) errors.push('failed-invocation');
  const usage = accounting(rows);
  if (usage.unknownUsageCalls) errors.push('unknown-usage');
  if (rows.some(call => !['maintenance', 'reasoning', 'action'].includes(call.stage))) errors.push('unknown-experiment-stage');
  return { arm: armId, scenarioId: scenario.id, status: errors.length ? 'unavailable' : 'completed', errors, usage };
}

function conditionErrors(calls) {
  const errors = [];
  for (const field of ['requested_model', 'reasoning', 'processing', 'cli_version', 'usage_scope', 'evaluationConditionsHash']) {
    const values = new Set(calls.map(call => call[field]).filter(value => value !== undefined && value !== null).map(value => JSON.stringify(value)));
    if (values.size > 1) errors.push(`mismatched-${field}`);
  }
  const returned = new Set(calls.map(call => call.returned_model ?? call.returnedModel).filter(Boolean));
  if (returned.size > 1) errors.push('mismatched-returned-model');
  return errors;
}

/** These compare matched execution workloads, not equivalent semantic quality. */
export function compareOverhead({ calls = [], results = [], arms = [], scenarios = [] }) {
  validateLedger(calls);
  const candidates = arms.filter(arm => !isStock(arm));
  const comparisons = [];
  for (const candidate of candidates) for (const baseline of arms.filter(isStock)) {
    const matched = [], excluded = [];
    for (const scenario of scenarios) {
      const left = scenarioScope({ calls, results, arm: candidate, scenario });
      const right = scenarioScope({ calls, results, arm: baseline, scenario });
      const scopeCalls = calls.filter(call => !shared(call) && call.scenarioId === scenario.id && [idOf(candidate), idOf(baseline)].includes(call.arm));
      const mismatches = conditionErrors(scopeCalls);
      left.errors.push(...mismatches); right.errors.push(...mismatches);
      if (left.errors.length || right.errors.length) {
        excluded.push({ scenarioId: scenario.id, candidateErrors: left.errors, baselineErrors: right.errors });
      } else matched.push({ scenarioId: scenario.id, candidate: left.usage, baseline: right.usage,
        ...tokenDifference(left.usage.completeTotalTokens, right.usage.completeTotalTokens) });
    }
    const matchedIds = new Set(matched.map(row => row.scenarioId));
    const selected = arm => calls.filter(call => !shared(call) && call.arm === idOf(arm) && matchedIds.has(call.scenarioId));
    const candidateUsage = accounting(selected(candidate)), baselineUsage = accounting(selected(baseline));
    comparisons.push({ candidateArm: idOf(candidate), baselineArm: idOf(baseline),
      scope: 'matched-completed-scenarios-including-maintenance-once',
      matchedScenarioIds: [...matchedIds], matchedScenarios: matched, excludedScenarios: excluded,
      candidateUsage, baselineUsage,
      exploratory: [...selected(candidate), ...selected(baseline)].some(call => !(call.returned_model ?? call.returnedModel)),
      ...(matched.length ? tokenDifference(candidateUsage.completeTotalTokens, baselineUsage.completeTotalTokens)
        : { status: 'unavailable', reason: 'no-matched-completed-workload', difference: null, ratio: null, percentageDifference: null }),
      qualityCaveat: 'Matched completed execution does not establish equal outcome quality; read H/Q/W/D and failures alongside usage.' });
  }
  return { measurementScope: MEASUREMENT_SCOPE, comparisons, usage: usageBreakdown({ calls, arms, scenarios }),
    limitations: ['CLI usage includes instructions, framing, and delivered tool context; it is not isolated memory-payload usage.',
      'Cached input and reasoning output are subsets. Native provider usage outside CLI reporting is not included.',
      'Mixed lookup-and-answer invocations are counted once in their recorded task stage, without a causal token split.'] };
}
