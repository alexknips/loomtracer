import { ratings, scoreH, scoreAction } from './scoring.mjs';
import { usageReport, compareOverhead } from './accounting.mjs';
import { armsForProfile, primaryTasksFor, sharedTasksFor } from './package-support.mjs';

export const REPORT_INTERPRETATION_VERSION = 'functionality-v5-report-1.1.0';
const renderRating=x=>x?.semanticStatus==='pending'?(x.provisionalRating===null?'pending':`${x.provisionalRating} provisional pending`):x?.rating??(x?.lower!==null&&x?.lower!==undefined?`${Math.round(x.lower)}–${Math.round(x.upper)} pending`:'unavailable');
const rowsOf = value => Object.values(value ?? {});
const caseId = row => row.caseId ?? row.scenarioId;
const hasPendingJudgments = grade => !grade || (grade.uncertainties ?? []).length > 0 ||
  [...(grade.hUnits ?? []), ...(grade.qUnits ?? []), ...(grade.wUnits ?? []), ...(grade.controlUnits ?? []),
    ...(grade.identity ? [grade.identity] : []), ...(grade.completion ? [grade.completion] : [])]
    .some(j => !['pass', 'fail'].includes(j.status));
const statusFromBoolean = value => value === true ? 'pass' : value === false ? 'fail' : 'pending';
const escapeCell = value => String(value ?? '—').replaceAll('|', '\\|').replaceAll('\n', ' ');

/** Frozen plan coverage, including packets not yet constructed at an early stop. */
export function gradingCoverage(scenarios, state, { profile = 'notes' } = {}) {
  const armCount = armsForProfile(profile).length;
  const entries = Object.entries(state.reviewPackets ?? {});
  const cases = scenarios.map(scenario => {
    const expectedByKind = { H: (armCount + 1) * Math.ceil(scenario.hUnits.length / 10), action: 2 * armCount + 1, withheld: 1 };
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

export function plannedWorkflows(scenarios, profile = 'notes') {
  return scenarios.flatMap(s => [
    {id:`${s.id}-acquisition`,kind:'acquisition',mode:'ordinary',control:'shared',arm:null},
    ...primaryTasksFor(s, profile), ...sharedTasksFor(s)
  ].map(task => ({...task, caseId:s.id})));
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

function pendingSemanticProjection(value, packetIds) {
  if (!packetIds.length) return value;
  const result=structuredClone(value);
  const mark=rating=>({...rating,provisionalRating:rating.rating, rating:null, semanticStatus:'pending',
    boundStatus:'provisional-before-global-uncertainty-resolution',pendingPacketIds:[...packetIds]});
  result.components=Object.fromEntries(Object.entries(result.components).map(([name,r])=>[name,mark(r)]));
  result.overall=mark(result.overall);
  for(const band of Object.values(result.perLevel)) {
    band.components=Object.fromEntries(Object.entries(band.components).map(([name,r])=>[name,mark(r)]));band.overall=mark(band.overall);
  }
  for(const row of result.cases) {
    row.semanticStatus='pending';row.pendingPacketIds=[...packetIds];
    for(const component of ['H','Q','W'])row[component]=mark(row[component]);
    row.H.provisionalCompleteChain=row.H.completeChain;
    if(row.H.completeChain==='pass')row.H.completeChain='pending';
    row.D={...row.D,provisionalStatus:row.D.status,semanticStatus:'pending',pendingPacketIds:[...packetIds],
      status:row.D.status==='pass'?'pending':row.D.status};
    row.provisionalCompleteTask=row.completeTask;
    if(row.completeTask==='pass')row.completeTask='pending';
  }
  result.semanticReview={status:'pending',packetIds:[...packetIds],reason:'At least one contributing grading packet contains unresolved global semantic uncertainty. Arm and case displays are conservatively provisional; raw unit judgments and established failures remain preserved.'};
  return result;
}
function uncertainPrimaryPackets(state, arm, profile, workflow = null) {
  return Object.entries(state.reviewPackets??{}).filter(([id,row])=>{
    if(row.control && row.control!=='primary')return false;
    if(workflow) {
      if(row.workflowIds?.length && !row.workflowIds.includes(workflow.id))return false;
      if(caseId(row)!==workflow.caseId || (row.packet??row).kind!==workflow.kind)return false;
      if(workflow.kind==='action' && row.packet?.participant?.mode && row.packet.participant.mode!==workflow.mode)return false;
    }
    if(row.arm?row.arm!==arm:profile!=='notes')return false;
    const grade=state.grades?.[id]?.grade??state.grades?.[id]?.value;
    return Boolean(grade?.uncertainties?.length);
  }).map(([id])=>id);
}

export function screenReport({manifest,scenarios,state,calls,stopReason=null}) {
  const results=rowsOf(state.results), usage=usageReport(calls), profile=manifest.profile??'notes', armIds=armsForProfile(profile);
  const arms=Object.fromEntries(armIds.map(arm=>[arm,{arm,modes:Object.fromEntries(['ordinary','prompted'].map(mode=>[mode,pendingSemanticProjection(ratings({scenarios,results,mode,arm}),uncertainPrimaryPackets(state,arm,profile))]))}]));
  const modes=arms['stock-maintained-notes'].modes;
  const workflows=rowsOf(state.workflows), plans=plannedWorkflows(scenarios,profile);
  const outcomes=plans.map(plan=>workflowOutcome(plan,workflows.find(w=>w.id===plan.id),scenarios.find(s=>s.id===plan.caseId),results));
  for(const outcome of outcomes) {
    const result=results.find(r=>r.id===outcome.id), directGrade=result?.grade?.grade??result?.grade?.value??result?.grade;
    if(outcome.control==='primary' && outcome.strictSuccess===true && (directGrade?.uncertainties?.length || uncertainPrimaryPackets(state,outcome.arm,profile,outcome).length)) {
      outcome.strictSuccess=null;outcome.strictOutcome='pending';outcome.pendingReason='Unresolved contributing semantic grading uncertainty';
    }
  }
  const grading=gradingCoverage(scenarios,state,{profile});
  const primary=workflows.filter(w=>w.control==='primary'&&w.kind!=='acquisition');
  const controls=results.filter(r=>['oracle','withheld'].includes(r.control)).map(r=>({id:r.id,caseId:r.caseId,kind:r.kind,control:r.control,status:r.status,grade:r.grade}));
  const unexpectedWorkflows=workflows.filter(w=>!plans.some(p=>p.id===w.id)).map(w=>w.id);
  const dispatchEnded=outcomes.filter(w=>w.dispatchEnded).length;
  const plannedResults=scenarios.length*(3*armIds.length+3);
  const recordedResults=results.filter(r=>scenarios.some(s=>s.id===caseId(r))).length;
  const pendingResults=Math.max(0,plannedResults-recordedResults);
  const cycleComplete=dispatchEnded===plans.length&&!unexpectedWorkflows.length&&grading.status==='model-review-complete'&&!pendingResults;
  const failed=outcomes.filter(w=>w.strictSuccess===false).length, pending=outcomes.filter(w=>w.strictSuccess===null).length;
  const status=stopReason?'stopped':!cycleComplete?'incomplete':failed?'completed-with-workflow-failures':pending?'completed-with-pending-outcomes':'completed';
  const rawSelection=state.auditSelection, selection=Array.isArray(rawSelection)?rawSelection:rawSelection?.selected??[];
  const humanAuditStatus=selection.length?'pending-human-review':'selection-pending';
  const componentFlags=Object.fromEntries(Object.entries(modes).map(([mode,r])=>[mode,Object.fromEntries(Object.entries(r.perLevel.demanding.components).map(([name,s])=>[name,s.rating===100?'ceiling':s.rating===0?'floor':s.rating===null?'pending':'mixed']))]));
  const overheadPlan=plans.filter(p=>p.control==='primary').map(p=>({...p,checkpointGroup:`${p.caseId}:${p.arm}`,maintenanceRequired:p.arm!=='stock-history'}));
  const overheadResults=outcomes.filter(o=>o.control==='primary').map(o=>({id:o.id,status:o.executionComplete?'completed':o.executionStatus,quality:o.strictOutcome==='fail'?'fail':uncertainPrimaryPackets(state,o.arm,profile).length?'pending':o.strictOutcome,humanAudit:o.humanAudit}));
  const overheadComparisons=profile==='comparison'?Object.fromEntries(['stock-maintained-notes','stock-history'].map(baseline=>[baseline,compareOverhead({calls,planned:overheadPlan.filter(p=>p.arm==='loomtracer-native'||p.arm===baseline),results:overheadResults.filter(r=>overheadPlan.some(p=>p.id===r.id&&(p.arm==='loomtracer-native'||p.arm===baseline))),candidate:'loomtracer-native',baseline})])):{status:'not-run',reason:'Notes profile contains maintained notes and shared controls only.'};
  const report={profile,arms,version:REPORT_INTERPRETATION_VERSION,evaluationReportVersion:manifest.versions.report,manifestHash:manifest.evaluationConditionsHash,
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
    overheadComparisons,
    exploratory:calls.some(c=>!c.returnedModel),fullSuiteRatings:null,
    caveats:['Three demanding development cases, one repetition. No full-suite or held-out claim. Paired differences are exploratory until semantic audit and repetition.',
      'A completed dispatch/review cycle does not mean every workflow succeeded. Execution status, terminal state and strict outcome are reported separately.',
      'Delivery-limited forks retain strict bounded-workflow failures. Their unexecuted milestones do not by themselves establish reasoning or memory loss.',
      'Strict semantic grades are model-reviewed and await stratified human audit. Missing, invalid and uncertain grades remain pending.',
      'Ordinary D measures expressed behavior; prompted D measures explicitly requested capability.',
      'Missing returned-model identity makes subscription comparisons exploratory.',
      'CLI-reported token usage includes instructions, framing, schemas and replay. Cached input and reasoning output are subsets.',
      'Failed and unknown-usage calls remain in the ledger; overlapping checkpoint views are not added.']};
  const table=[];
  for(const [arm,data] of Object.entries(arms)) for(const [mode,r] of Object.entries(data.modes)) {
    const band=r.perLevel.demanding;
    table.push(`| ${arm} | ${mode} | ${['H','Q','W','D'].map(k=>renderRating(band.components[k])).join(' | ')} | ${renderRating(band.overall)} |`);
  }
  const cases=[];
  for(const [arm,data] of Object.entries(arms)) for(const [mode,r] of Object.entries(data.modes)) for(const c of r.cases) {
    const execution=outcomes.find(w=>w.caseId===c.id&&w.kind==='action'&&w.control==='primary'&&w.mode===mode&&w.arm===arm);
    cases.push(`| ${c.id} | ${arm} | ${mode} | ${c.H.counts.pass}/${c.H.counts.planned} | ${renderRating(c.H)} | ${renderRating(c.Q)} | ${renderRating(c.W)} | ${c.D.status} | ${c.H.completeChain} | ${c.completeTask} | ${execution.executionStatus} | ${execution.terminalState??'none'} |`);
  }
  report.markdown=[`# V5 ${profile} calibration`,'',`Report status: ${report.status}${stopReason?` — ${stopReason}`:''}. Dispatch/review cycle: ${report.dispatchCycleStatus}. Human audit: ${report.humanAudit.status}.`,
    '',`${report.completion.strictPassedWorkflows}/${plans.length} workflows have provisional strict passes; ${failed} strict failures and ${pending} pending outcomes. ${report.completion.deliveryLimitedWorkflows} stopped at the delivery allowance.`,
    '', '| Arm | Mode | H | Q | W | D | Overall |','|---|---|---:|---:|---:|---:|---:|',...table,
    '', 'Ratings describe the demanding screen under its delivery and operation bounds. A failure to execute the remaining work is not, by itself, a finding of lost memory. Semantic results await human audit.',
    '', '| Case | Arm | Mode | Recorded H unit passes | H | Q | W | D decision | Complete chain | Complete task | Action execution | Terminal state |',
    '|---|---|---|---:|---:|---:|---:|---|---|---|---|---|',...cases,
    '', '## Workflow execution and completion','', '| Workflow | Execution | Terminal state | Strict outcome | Operations | Historical bytes | Recorded errors |',
    '|---|---|---|---|---:|---:|---|',
    ...outcomes.map(w=>`| ${w.id} | ${w.executionStatus} | ${w.terminalState??'none'} | ${w.strictOutcome} | ${w.operationsUsed??'—'} | ${w.historicalBytes??'—'} | ${escapeCell(w.errors.join('; ')||'none')} |`),
    '', '## Review coverage','',`${grading.reviewedGrades}/${grading.planned} planned packets have definitive model judgments. Missing: ${grading.missingGrades}; invalid: ${grading.invalidGrades}; semantically pending: ${grading.pendingSemanticGrades}. Merged result records still missing: ${pendingResults}.`,
    '', `Human audit: ${humanAuditStatus}; ${selection.length} packets selected, at least ${report.humanAudit.minimumPackets} required before considering additional uncertain, disputed or consequential judgments. An empty selection means selection is pending.`,
    '', '## CLI-reported token usage','', '| Stage | Calls | Known tokens | Unknown calls |','|---|---:|---:|---:|',
    ...Object.entries(usage.stages).map(([stage,u])=>`| ${stage} | ${u.calls} | ${u.reportedTokens.toLocaleString('en-US')} | ${u.unknownUsageCalls} |`),
    '',`Whole screen: **${usage.total.reportedTokens.toLocaleString('en-US')} known tokens / ${manifest.budgetTokens.toLocaleString('en-US')} approved ceiling**, ${usage.total.unknownUsageCalls} calls with unknown usage.`,
    '',`Post-handoff primary workload: ${usage.primary.reportedTokens.toLocaleString('en-US')} known tokens. Shared acquisition, controls, grading and canaries are excluded from arm overhead and retained in whole-screen expenditure.`,
    '', '| Arm | Calls | Known input | Known output | Known total | Known cached input | Known reasoning output | Known uncached input | Unknown total calls |', '|---|---:|---:|---:|---:|---:|---:|---:|---:|',
    ...armIds.map(arm=>{const u=usage.arms[arm];return `| ${arm} | ${u?.calls??0} | ${u?.tokens.input.knownSubtotal??0} | ${u?.tokens.output.knownSubtotal??0} | ${u?.reportedTokens??0} | ${u?.tokens.cachedInput.knownSubtotal??0} | ${u?.tokens.reasoningOutput.knownSubtotal??0} | ${u?.tokens.uncachedInput.knownSubtotal??0} | ${u?.unknownUsageCalls??0} |`;}),
    '', '## Matched token overhead', '',
    ...(profile==='comparison'?['| Loomtracer relative to | Status | Additional tokens | Usage ratio | Percentage overhead |', '|---|---|---:|---:|---:|', ...Object.entries(overheadComparisons).map(([baseline,c])=>`| ${baseline} | ${c.status}${c.exploratory?' exploratory':''} | ${c.differences?.difference??'unavailable'} | ${c.differences?.ratio??'unavailable'} | ${c.differences?.percentageDifference??'unavailable'} |`), ...Object.entries(overheadComparisons).filter(([,c])=>c.errors.length).map(([baseline,c])=>`${baseline}: ${c.errors.join('; ')}.`), 'These are matched execution costs, not claims of equal successful work. Read strict quality and completion above; human review remains pending.']:['No paired overhead is available in the notes-only profile.']),
    '', '## Interpretation limits','',...report.caveats.map(c=>`- ${c}`),''].join('\n');
  return report;
}
