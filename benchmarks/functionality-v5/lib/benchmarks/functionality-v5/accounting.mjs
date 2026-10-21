/** V5 expenditure is an immutable invocation ledger, never a sum of overlapping views. */
import { accounting as baseAccounting, normalizedUsage, tokenDifference } from '../functionality/accounting.mjs';

export { normalizedUsage, tokenDifference };
export const MEASUREMENT_SCOPE = 'functionality-v5-cli-reported-invocations-1';
export const LABEL = 'CLI-reported token usage';
export const STAGES = ['acquisition', 'maintenance', 'reasoning', 'H', 'action', 'retrieval', 'oracle', 'withheld', 'grading', 'canary'];
export const PRIMARY_STAGES = ['maintenance', 'reasoning', 'H', 'action', 'retrieval'];
export const SHARED_STAGES = ['acquisition', 'oracle', 'withheld', 'grading', 'canary'];
const primary = c => PRIMARY_STAGES.includes(c.stage);
const canonicalStage = stage => stage === 'H' ? 'reasoning' : stage;
const caseId = c => c.scenarioId ?? c.caseId;

export function accounting(calls = []) {
  if (!Array.isArray(calls) || calls.some(c => !STAGES.includes(c.stage))) throw new Error('Known invocation stage required');
  return { ...baseAccounting(calls), measurementScope: MEASUREMENT_SCOPE, label: LABEL };
}

export function usageReport(calls = []) {
  const total = accounting(calls);
  const stages = Object.fromEntries([...new Set(STAGES.map(canonicalStage))].map(stage =>
    [stage, accounting(calls.filter(c => canonicalStage(c.stage) === stage))]));
  const armIds = [...new Set(calls.filter(primary).map(c => c.arm).filter(Boolean))];
  const arms = Object.fromEntries(armIds.map(arm => [arm, accounting(calls.filter(c => primary(c) && c.arm === arm))]));
  const scenarios = Object.fromEntries([...new Set(calls.map(caseId).filter(Boolean))].map(id => {
    const rows = calls.filter(c => caseId(c) === id);
    return [id, { total: accounting(rows), stages: Object.fromEntries([...new Set(rows.map(c => canonicalStage(c.stage)))].map(stage =>
      [stage, accounting(rows.filter(c => canonicalStage(c.stage) === stage))])),
    arms: Object.fromEntries(armIds.map(arm => [arm, accounting(rows.filter(c => primary(c) && c.arm === arm))])) }];
  }));
  const acquisition = accounting(calls.filter(c => c.stage === 'acquisition'));
  return { measurementScope: MEASUREMENT_SCOPE, label: LABEL, total, stages, arms, scenarios,
    acquisition, shared: accounting(calls.filter(c => !primary(c))),
    primary: accounting(calls.filter(primary)), unassignedPrimary: accounting(calls.filter(c => primary(c) && !c.arm)),
    caveats: [
      'Input includes instructions, framing, schemas, consumed context and replay; it is not isolated memory-payload usage.',
      'Cached input and reasoning output are subsets, never additional tokens. No bytes-to-tokens or dollar conversion.',
      'Acquisition, canaries, controls and grading count once in whole-experiment expenditure and are excluded from post-handoff arm overhead.',
      'Shared maintenance and mixed lookup-and-task invocations count once. Q/W/D share one action cost.',
      'Views overlap and must not be added. Failed calls remain included; unknown usage is not zero.'
    ] };
}

export function dispatchStatus(calls, budget) {
  if (!Number.isSafeInteger(budget) || budget <= 0) throw new Error('Explicit positive integer budget required');
  const usage = accounting(calls), reason = usage.unknownUsageCalls ? 'unknown-usage' : usage.reportedTokens >= budget ? 'threshold-reached' : null;
  return { mayDispatch: !reason, reason, usage, remainingKnownTokens: budget - usage.reportedTokens,
    note: 'Stops subsequent dispatch, not an in-flight provider call. Unknown usage cannot be charged as zero.' };
}

/** Matched execution scope, with quality shown separately; no cheaper-success claim from failed work. */
export function compareOverhead({ calls = [], planned = [], results = [], candidate, baseline }) {
  accounting(calls);
  if (!candidate || !baseline || candidate === baseline) throw new Error('Two distinct comparison arms required');
  if (new Set(planned.map(p => p.id)).size !== planned.length || new Set(results.map(r => r.id)).size !== results.length) throw new Error('Unique planned/result identifiers required');
  const resultById = new Map(results.map(r => [r.id, r])), selected = {}, errors = [];
  const signature = p => `${caseId(p)}:${p.kind ?? 'unspecified'}:${p.mode ?? 'prompted'}:${p.repetition ?? 1}`;
  for (const arm of [candidate, baseline]) {
    const trials = planned.filter(p => p.arm === arm), groups = new Set(trials.map(p => p.checkpointGroup).filter(Boolean));
    selected[arm] = calls.filter(c => primary(c) && c.arm === arm &&
      (c.stage === 'maintenance' ? groups.has(c.checkpointGroup) : trials.some(p => p.id === c.trialId)));
    if (!trials.length || trials.some(p => resultById.get(p.id)?.status !== 'completed')) errors.push(`${arm}:incomplete-workload`);
    if (trials.some(p => !selected[arm].some(c => c.trialId === p.id && ['reasoning', 'H', 'action'].includes(c.stage)))) errors.push(`${arm}:missing-task-invocation`);
    if (trials.some(p => p.maintenanceRequired && !selected[arm].some(c => c.stage === 'maintenance' && c.checkpointGroup === p.checkpointGroup))) errors.push(`${arm}:missing-maintenance`);
    if (selected[arm].some(c => c.status !== 'completed')) errors.push(`${arm}:failed-invocation`);
    if (accounting(selected[arm]).unknownUsageCalls) errors.push(`${arm}:unknown-usage`);
  }
  const signatures = arm => planned.filter(p => p.arm === arm).map(signature).sort().join('|');
  if (signatures(candidate) !== signatures(baseline)) errors.push('unmatched-scenario-mode-repetition-set');
  const rows = [...selected[candidate], ...selected[baseline]];
  for (const field of ['evaluationConditionsHash', 'requestedModel', 'reasoning', 'cliVersion', 'measurementScope']) {
    if (rows.some(c => !c[field]) || new Set(rows.map(c => c[field])).size !== 1) errors.push(`missing-or-mismatched-${field}`);
  }
  if (rows.some(c => c.measurementScope !== MEASUREMENT_SCOPE)) errors.push('incompatible-report-measurement-scope');
  if (new Set(rows.map(c => c.returnedModel).filter(Boolean)).size > 1) errors.push('mismatched-returned-model');
  const left = accounting(selected[candidate]), right = accounting(selected[baseline]);
  return { status: errors.length ? 'unavailable' : 'available', measurementScope: MEASUREMENT_SCOPE,
    scope: 'matched-post-handoff-maintenance-plus-H-and-QWD', errors, candidate: left, baseline: right,
    differences: errors.length ? null : tokenDifference(left.completeTotalTokens, right.completeTotalTokens),
    exploratory: rows.some(c => !c.returnedModel),
    quality: planned.map(p => ({ trialId: p.id, status: resultById.get(p.id)?.status ?? 'missing', quality: resultById.get(p.id)?.quality ?? 'pending' })),
    caveat: 'Equal completed execution scope does not establish equal quality. Read strict component ratings and completion beside expenditure.' };
}

/** Non-additive checkpoint view: the underlying ledger remains the source of totals. */
export function checkpointUsage(calls, { arm, scenarioId, captureEpisode, trialIds = [] }) {
  accounting(calls);
  const selected = calls.filter(c => c.arm === arm && caseId(c) === scenarioId);
  return { nonAdditive: true,
    maintenanceToDate: accounting(selected.filter(c => c.stage === 'maintenance' && (c.episode ?? 0) <= captureEpisode)),
    currentExecution: accounting(selected.filter(c => trialIds.includes(c.trialId) && ['reasoning', 'H', 'action', 'retrieval'].includes(c.stage))),
    controls: accounting(calls.filter(c => caseId(c) === scenarioId && ['oracle', 'withheld'].includes(c.stage) && trialIds.includes(c.trialId))) };
}
