import { ratings, scoreH, scoreAction } from './scoring.mjs';
import { usageReport } from './accounting.mjs';

export const REPORT_INTERPRETATION_VERSION = 'functionality-v5-report-1.0.2';
const renderRating=x=>x?.rating??(x?.lower!==null&&x?.lower!==undefined?`${Math.round(x.lower)}–${Math.round(x.upper)} pending`:'unavailable');
const rowsOf = value => Object.values(value ?? {});
const caseId = row => row.caseId ?? row.scenarioId;
const hasPendingJudgments = grade => !grade || (grade.uncertainties ?? []).length > 0 ||
  [...(grade.hUnits ?? []), ...(grade.qUnits ?? []), ...(grade.wUnits ?? []), ...(grade.controlUnits ?? []),
    ...(grade.identity ? [grade.identity] : []), ...(grade.completion ? [grade.completion] : [])]
    .some(j => !['pass', 'fail'].includes(j.status));
const statusFromBoolean = value => value === true ? 'pass' : value === false ? 'fail' : 'pending';
const escapeCell = value => String(value ?? '—').replaceAll('|', '\\|').replaceAll('\n', ' ');

/** Frozen plan coverage, including packets not yet constructed at an early stop. */
export function gradingCoverage(scenarios, state) {
  const entries = Object.entries(state.reviewPackets ?? {});
  const cases = scenarios.map(scenario => {
    const expectedByKind = { H: 2 * Math.ceil(scenario.hUnits.length / 10), action: 3, withheld: 1 };
    const selected = entries.filter(([, row]) => caseId(row) === scenario.id);
    const counts = { planned: Object.values(expectedByKind).reduce((n, v) => n + v, 0), recordedPackets: 0,
      missingPackets: 0, missingGradeRecords: 0, invalidGrades: 0, pendingSemanticGrades: 0, reviewedGrades: 0, unexpectedPackets: 0 };
    const details = [];
    for (const [kind, expected] of Object.entries(expectedByKind)) {
      const matches = selected.filter(([, row]) => (row.packet ?? row).kind === kind);
      counts.missingPackets += Math.max(0, expected - matches.length);
      counts.unexpectedPackets += Math.max(0, matches.length - expected);
      for (const [id, row] of matches.slice(0, expected)) {
        counts.recordedPackets++;
        const record = state.grades?.[id], grade = record?.grade ?? record?.value;
        let status;
        if (!record) { counts.missingGradeRecords++; status = 'missing-grade'; }
        else if (record.status !== 'model-reviewed' || !grade || (record.errors ?? []).length || grade.blindId !== (row.packet ?? row).blindId) {
          counts.invalidGrades++; status = 'invalid-grade';
        } else if (hasPendingJudgments(grade)) { counts.pendingSemanticGrades++; status = 'pending-semantic-grade'; }
        else { counts.reviewedGrades++; status = 'model-reviewed'; }
        details.push({ id, kind, status, errors: record?.errors ?? [] });
      }
    }
    counts.unexpectedPackets += selected.filter(([, row]) => !Object.hasOwn(expectedByKind, (row.packet ?? row).kind)).length;
    return { caseId: scenario.id, expectedByKind, ...counts, missingGrades: counts.missingPackets + counts.missingGradeRecords,
      pendingGradePackets: counts.missingPackets + counts.missingGradeRecords + counts.invalidGrades + counts.pendingSemanticGrades, details };
  });
  const countFields = ['planned', 'recordedPackets', 'missingPackets', 'missingGradeRecords', 'missingGrades', 'invalidGrades', 'pendingSemanticGrades', 'reviewedGrades', 'pendingGradePackets', 'unexpectedPackets'];
  const totals = Object.fromEntries(countFields.map(field => [field, cases.reduce((sum, row) => sum + row[field], 0)]));
  totals.unexpectedPackets += entries.filter(([, row]) => !scenarios.some(s => s.id === caseId(row))).length;
  return { ...totals, cases, status: totals.pendingGradePackets || totals.unexpectedPackets ? 'incomplete' : 'model-review-complete', humanAudit: 'pending' };
}

function plannedWorkflows(scenarios) {
  return scenarios.flatMap(s => [
    ['acquisition','acquisition','ordinary','shared'], ['H','H','prompted','primary'],
    ['ordinary','action','ordinary','primary'], ['prompted','action','prompted','primary'],
    ['oracle-H','H','prompted','oracle'], ['oracle-action','action','ordinary','oracle'],
    ['withheld-H','H','prompted','withheld'], ['withheld-action','action','ordinary','withheld']
  ].map(([suffix,kind,mode,control]) => ({ id:`${s.id}-${suffix}`, caseId:s.id, kind, mode, control })));
}

function workflowOutcome(plan, workflow, scenario, results) {
  const executionStatus = workflow?.status ?? 'not-run';
  const dispatchEnded = !['not-run', 'running', 'active'].includes(executionStatus);
  const executionComplete = ['completed', 'blocked'].includes(executionStatus);
  const result = results.find(r => r.id === plan.id);
  let strictSuccess = null;
  if (dispatchEnded && !executionComplete) strictSuccess = false;
  else if (!executionComplete) strictSuccess = null;
  else if (plan.kind === 'acquisition') strictSuccess = workflow?.deterministic?.complete ?? null;
  else if (plan.control === 'withheld') {
    const paired = results.find(r => caseId(r) === plan.caseId && r.control === 'withheld' && r.kind === 'withheld-pair');
    const judgment = paired?.grade?.controlUnits?.find(u => u.id === (plan.kind === 'H' ? 'withheld-H' : 'withheld-action'));
    if (workflow?.deterministic?.complete === false) strictSuccess = false;
    else if (executionComplete && ['pass','fail'].includes(judgment?.status)) strictSuccess = judgment.status === 'pass';
  } else if (plan.kind === 'H' && result?.grade) {
    const h = scoreH(scenario.hUnits, result.grade.hUnits ?? [], { unsupportedClaims: result.grade.unsupportedClaims ?? [] });
    strictSuccess = h.completeChain === 'pending' ? null : h.completeChain === 'pass';
  } else if (plan.kind === 'action') {
    const action = scoreAction(scenario, result?.grade ?? {}, { deterministic: workflow?.deterministic ?? result?.deterministic ?? {}, mode: plan.mode });
    strictSuccess = action.completeTask === 'pending' ? null : action.completeTask === 'pass';
  }
  const terminal = workflow?.deterministic?.terminal ?? workflow?.world?.terminal ?? null;
  const errors = [...(workflow?.errors ?? []), ...(workflow?.deterministic?.errors ?? []).map(e => `${e.code}: ${e.detail}`)];
  return { ...plan, executionStatus, dispatchEnded, executionComplete,
    terminalState: terminal?.status ?? (plan.kind === 'H' && executionStatus === 'completed' ? 'answer-returned' : null),
    strictSuccess, strictOutcome: statusFromBoolean(strictSuccess), humanAudit: plan.kind === 'acquisition' ? 'not-applicable' : 'pending',
    resolvedWork: workflow?.deterministic?.resolvedWork ?? null, correctlyBlocked: workflow?.deterministic?.correctlyBlocked ?? null,
    operationsUsed: workflow?.operationsUsed ?? null, calls: workflow?.callIds?.length ?? 0,
    historicalBytes: workflow?.view?.historicalBytes ?? null,
    errors, observedTermination: executionStatus === 'delivery-limit' ? { type: 'delivery-limit', errors: workflow?.errors ?? [],
      interpretation: 'The fork stopped at its recorded delivery allowance. Unexecuted tail milestones fail the bounded workflow; this alone does not establish lost memory.' } : null,
    firstLoss: 'not established' };
}

export function screenReport({manifest,scenarios,state,calls,stopReason=null}) {
  const results=rowsOf(state.results), usage=usageReport(calls);
  const modes=Object.fromEntries(['ordinary','prompted'].map(mode=>[mode,ratings({scenarios,results,mode})]));
  const workflows=rowsOf(state.workflows), plans=plannedWorkflows(scenarios);
  const outcomes=plans.map(plan=>workflowOutcome(plan,workflows.find(w=>w.id===plan.id),scenarios.find(s=>s.id===plan.caseId),results));
  const grading=gradingCoverage(scenarios,state);
  const primary=workflows.filter(w=>w.control==='primary'&&w.kind!=='acquisition');
  const controls=results.filter(r=>['oracle','withheld'].includes(r.control)).map(r=>({id:r.id,caseId:r.caseId,kind:r.kind,control:r.control,status:r.status,grade:r.grade}));
  const unexpectedWorkflows=workflows.filter(w=>!plans.some(p=>p.id===w.id)).map(w=>w.id);
  const dispatchEnded=outcomes.filter(w=>w.dispatchEnded).length;
  const plannedResults=scenarios.length*6;
  const recordedResults=results.filter(r=>scenarios.some(s=>s.id===caseId(r))).length;
  const pendingResults=Math.max(0,plannedResults-recordedResults);
  const cycleComplete=dispatchEnded===plans.length&&!unexpectedWorkflows.length&&grading.status==='model-review-complete'&&!pendingResults;
  const failed=outcomes.filter(w=>w.strictSuccess===false).length, pending=outcomes.filter(w=>w.strictSuccess===null).length;
  const status=stopReason?'stopped':!cycleComplete?'incomplete':failed?'completed-with-workflow-failures':pending?'completed-with-pending-outcomes':'completed';
  const rawSelection=state.auditSelection, selection=Array.isArray(rawSelection)?rawSelection:rawSelection?.selected??[];
  const humanAuditStatus=selection.length?'pending-human-review':'selection-pending';
  const componentFlags=Object.fromEntries(Object.entries(modes).map(([mode,r])=>[mode,Object.fromEntries(Object.entries(r.perLevel.demanding.components).map(([name,s])=>[name,s.rating===100?'ceiling':s.rating===0?'floor':s.rating===null?'pending':'mixed']))]));
  const report={version:REPORT_INTERPRETATION_VERSION,evaluationReportVersion:manifest.versions.report,manifestHash:manifest.evaluationConditionsHash,
    status,dispatchCycleStatus:stopReason?'stopped':cycleComplete?'completed':'incomplete',
    stopReason,budgetTokens:manifest.budgetTokens,usage,remainingKnownTokens:manifest.budgetTokens-usage.total.reportedTokens,
    completion:{plannedWorkflows:plans.length,recordedWorkflows:workflows.length,endedDispatches:dispatchEnded,
      terminalWorkflows:outcomes.filter(w=>w.executionComplete).length,strictPassedWorkflows:outcomes.filter(w=>w.strictSuccess===true).length,
      strictFailedWorkflows:failed,pendingOutcomeWorkflows:pending,primaryForks:primary.length,unexpectedWorkflows,
      deliveryLimitedWorkflows:outcomes.filter(w=>w.executionStatus==='delivery-limit').length,
      acquisitions:outcomes.filter(w=>w.kind==='acquisition').map(w=>({caseId:w.caseId,status:w.executionStatus,terminalState:w.terminalState,success:w.strictSuccess})),
      plannedResultRecords:plannedResults,recordedResultRecords:recordedResults,pendingResultRecords:pendingResults,
      pendingGradePackets:grading.pendingGradePackets},workflowOutcomes:outcomes,grading,modes,componentFlags,controls,
    citationCompliance:workflows.map(w=>({id:w.id,emitted:w.citations?.length??0,valid:w.citations?.filter(c=>c.valid).length??0})),
    humanAudit:{status:humanAuditStatus,minimumFraction:.25,minimumPackets:Math.ceil(grading.planned*.25),selection,
      selectionStatus:selection.length?'selected':'pending',allUncertainDisputedAndConsequentialRequired:true},
    overheadComparisons:{status:'not-run',reason:'This approved screen contains maintained notes and controls only; no matched native/history arm exists.'},
    exploratory:calls.some(c=>!c.returnedModel),fullSuiteRatings:null,
    caveats:['Three demanding development cases, one repetition. No full-suite, held-out, stock/native or version-benefit claim.',
      'A completed dispatch/review cycle does not mean every workflow succeeded. Execution status, terminal state and strict outcome are reported separately.',
      'Delivery-limited forks retain strict bounded-workflow failures. Their unexecuted milestones do not by themselves establish reasoning or memory loss.',
      'Strict semantic grades are model-reviewed and await stratified human audit. Missing, invalid and uncertain grades remain pending.',
      'Ordinary D measures expressed behavior; prompted D measures explicitly requested capability.',
      'Missing returned-model identity makes subscription comparisons exploratory.',
      'CLI-reported token usage includes instructions, framing, schemas and replay. Cached input and reasoning output are subsets.',
      'Failed and unknown-usage calls remain in the ledger; overlapping checkpoint views are not added.']};
  const table=[];
  for(const [mode,r] of Object.entries(modes)) {
    const band=r.perLevel.demanding;
    table.push(`| ${mode} | ${['H','Q','W','D'].map(k=>renderRating(band.components[k])).join(' | ')} | ${renderRating(band.overall)} |`);
  }
  const cases=[];
  for(const [mode,r] of Object.entries(modes)) for(const c of r.cases) {
    const execution=outcomes.find(w=>w.caseId===c.id&&w.kind==='action'&&w.control==='primary'&&w.mode===mode);
    cases.push(`| ${c.id} | ${mode} | ${c.H.counts.pass}/${c.H.counts.planned} | ${renderRating(c.H)} | ${renderRating(c.Q)} | ${renderRating(c.W)} | ${c.D.status} | ${c.H.completeChain} | ${c.completeTask} | ${execution.executionStatus} | ${execution.terminalState??'none'} |`);
  }
  report.markdown=['# V5 maintained-notes calibration','',`Report status: ${report.status}${stopReason?` — ${stopReason}`:''}. Dispatch/review cycle: ${report.dispatchCycleStatus}. Human audit: ${report.humanAudit.status}.`,
    '',`${report.completion.strictPassedWorkflows}/${plans.length} workflows have provisional strict passes; ${failed} strict failures and ${pending} pending outcomes. ${report.completion.deliveryLimitedWorkflows} stopped at the delivery allowance.`,
    '', '| Mode | H | Q | W | D | Overall |','|---|---:|---:|---:|---:|---:|',...table,
    '', 'Ratings describe the demanding screen under its delivery and operation bounds. A failure to execute the remaining work is not, by itself, a finding of lost memory. Semantic results await human audit.',
    '', '| Case | Mode | H units passed | H | Q | W | D decision | Complete chain | Complete task | Action execution | Terminal state |',
    '|---|---|---:|---:|---:|---:|---|---|---|---|---|',...cases,
    '', '## Workflow execution and completion','', '| Workflow | Execution | Terminal state | Strict outcome | Operations | Historical bytes | Recorded errors |',
    '|---|---|---|---|---:|---:|---|',
    ...outcomes.map(w=>`| ${w.id} | ${w.executionStatus} | ${w.terminalState??'none'} | ${w.strictOutcome} | ${w.operationsUsed??'—'} | ${w.historicalBytes??'—'} | ${escapeCell(w.errors.join('; ')||'none')} |`),
    '', '## Review coverage','',`${grading.reviewedGrades}/${grading.planned} planned packets have definitive model judgments. Missing: ${grading.missingGrades}; invalid: ${grading.invalidGrades}; semantically pending: ${grading.pendingSemanticGrades}. Merged result records still missing: ${pendingResults}.`,
    '', `Human audit: ${humanAuditStatus}; ${selection.length} packets selected, at least ${report.humanAudit.minimumPackets} required before considering additional uncertain, disputed or consequential judgments. An empty selection means selection is pending.`,
    '', '## CLI-reported token usage','', '| Stage | Calls | Known tokens | Unknown calls |','|---|---:|---:|---:|',
    ...Object.entries(usage.stages).map(([stage,u])=>`| ${stage} | ${u.calls} | ${u.reportedTokens.toLocaleString('en-US')} | ${u.unknownUsageCalls} |`),
    '',`Whole screen: **${usage.total.reportedTokens.toLocaleString('en-US')} known tokens / ${manifest.budgetTokens.toLocaleString('en-US')} approved ceiling**, ${usage.total.unknownUsageCalls} calls with unknown usage.`,
    '',`Post-handoff primary workload: ${usage.primary.reportedTokens.toLocaleString('en-US')} known tokens. No native/history overhead ratio is available from this notes-only screen.`,
    '', '## Interpretation limits','',...report.caveats.map(c=>`- ${c}`),''].join('\n');
  return report;
}
