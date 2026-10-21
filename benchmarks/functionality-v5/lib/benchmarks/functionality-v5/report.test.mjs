import test from 'node:test';
import assert from 'node:assert/strict';
import { buildCorpus, selectedScreenIds } from './corpus.mjs';
import { screenReport, gradingCoverage, REPORT_INTERPRETATION_VERSION } from './report.mjs';
import { usageReport } from './accounting.mjs';
import { VERSIONS } from './common.mjs';
import { REPORT_VERSION, SCORING_VERSION } from './scoring.mjs';

const pass = id => ({ id, status:'pass', completeUnambiguous:true, supported:true, materialContradiction:false });
function fixture() {
  const scenarios=buildCorpus().filter(s=>selectedScreenIds.includes(s.id));
  const manifest={versions:{...VERSIONS},evaluationConditionsHash:'frozen-conditions',budgetTokens:4250000};
  const state={workflows:{},results:{},reviewPackets:{},grades:{}};
  for(const s of scenarios) {
    const hGrade={hUnits:s.hUnits.map(u=>pass(u.id)),unsupportedClaims:[]};
    const actionGrade={qUnits:s.qUnits.map(u=>pass(u.id)),wUnits:s.wUnits.map(u=>pass(u.id)),identity:{...pass('identity'),observed:true,late:false},completion:{status:'pass'},unsupportedClaims:[]};
    const definitions=[['acquisition','acquisition','ordinary','shared'],['H','H','prompted','primary'],['ordinary','action','ordinary','primary'],['prompted','action','prompted','primary'],['oracle-H','H','prompted','oracle'],['oracle-action','action','ordinary','oracle'],['withheld-H','H','prompted','withheld'],['withheld-action','action','ordinary','withheld']];
    for(const [suffix,kind,mode,control] of definitions) {
      const id=`${s.id}-${suffix}`,blocked=kind==='action'&&(control==='withheld'||s.identity==='unresolved');
      const deterministic=kind==='H'?null:{complete:true,resolvedWork:!blocked,correctlyBlocked:blocked,terminal:{status:blocked?'blocked':'complete'},errors:[],Q:s.qUnits.map(u=>({...pass(u.id),kind:u.kind})),W:s.wUnits.map(u=>({...pass(u.id),kind:u.kind})),D:{observed:true,late:false}};
      state.workflows[id]={id,caseId:s.id,kind,mode,control,status:blocked?'blocked':'completed',deterministic,operationsUsed:8,callIds:[`${id}-call`],errors:[],view:{historicalBytes:9000}};
      if(kind!=='acquisition'&&control!=='withheld')state.results[id]={id,caseId:s.id,kind,mode,control,status:state.workflows[id].status,grade:structuredClone(kind==='H'?hGrade:actionGrade),deterministic};
    }
    const controlId=`${s.id}-withheld-control`;
    state.results[controlId]={id:controlId,caseId:s.id,kind:'withheld-pair',control:'withheld',status:'completed',grade:{controlUnits:[pass('withheld-H'),pass('withheld-action')]}};
    let number=0;
    for(const kind of ['H','H','H','H','action','action','action','withheld']) {
      const blindId=`${s.id}-review-${++number}`;
      state.reviewPackets[blindId]={caseId:s.id,family:s.family,packet:{blindId,kind}};
      state.grades[blindId]={status:'model-reviewed',errors:[],grade:{blindId,hUnits:[pass('fixture')],uncertainties:[],unsupportedClaims:[]}};
    }
  }
  state.auditSelection={selected:Object.keys(state.reviewPackets).slice(0,6).map(blindId=>({blindId})),status:'pending-human-review'};
  const calls=[{id:'known-failed',stage:'action',arm:'stock-maintained-notes',caseId:scenarios[0].id,status:'failed',usage:{input_tokens:100,output_tokens:20,cached_input_tokens:30,reasoning_output_tokens:5}},
    {id:'shared',stage:'canary',status:'completed',usage:{total_tokens:80}}];
  return {manifest,scenarios,state,calls};
}

test('clean completed work distinguishes dispatch completion, strict outcomes and pending human audit',()=>{
  const input=fixture(),r=screenReport(input);
  assert.equal(r.version,REPORT_INTERPRETATION_VERSION);assert.equal(r.evaluationReportVersion,'functionality-v5-report-1.1.0');
  assert.equal(r.version,VERSIONS.report);assert.equal(r.version,REPORT_VERSION);
  for(const mode of ['ordinary','prompted']) {
    assert.equal(r.modes[mode].version,REPORT_VERSION);
    assert.equal(r.modes[mode].scoringVersion,SCORING_VERSION);
  }
  assert.equal(r.status,'completed');assert.equal(r.dispatchCycleStatus,'completed');
  assert.equal(r.completion.plannedWorkflows,24);assert.equal(r.completion.strictPassedWorkflows,24);
  assert.equal(r.completion.strictFailedWorkflows,0);assert.equal(r.grading.reviewedGrades,24);
  assert.equal(r.humanAudit.status,'pending-human-review');assert.equal(r.humanAudit.minimumPackets,6);
  assert.equal(r.fullSuiteRatings,null);
});
test('a new report projection preserves the evaluation version of older saved inputs',()=>{
  const input=fixture();input.manifest.versions.report='functionality-v5-report-1.0.0';
  const before=structuredClone(input),r=screenReport(input);
  assert.equal(r.version,'functionality-v5-report-1.1.0');
  assert.equal(r.evaluationReportVersion,'functionality-v5-report-1.0.0');
  assert.deepEqual(input,before);
  assert.deepEqual(r.usage,usageReport(input.calls));
});
test('delivery-limited workflows cannot be presented as successful completed work',()=>{
  const input=fixture(),id=`${input.scenarios[0].id}-ordinary`,w=input.state.workflows[id];
  w.status='delivery-limit';w.errors=['32 KiB cumulative historical delivery allowance exhausted'];w.view.historicalBytes=24152;w.deterministic.complete=false;w.deterministic.terminal=null;
  w.deterministic.W[0].status='fail';input.state.results[id].status='delivery-limit';
  const r=screenReport(input),outcome=r.workflowOutcomes.find(x=>x.id===id);
  assert.equal(r.dispatchCycleStatus,'completed');assert.equal(r.status,'completed-with-workflow-failures');
  assert.equal(r.completion.strictPassedWorkflows,23);assert.equal(r.completion.strictFailedWorkflows,1);assert.equal(r.completion.terminalWorkflows,23);
  assert.equal(outcome.executionStatus,'delivery-limit');assert.equal(outcome.terminalState,null);assert.equal(outcome.strictSuccess,false);
  assert.equal(outcome.observedTermination.type,'delivery-limit');assert.equal(outcome.firstLoss,'not established');
  assert.match(r.markdown,/delivery-limit/);assert.match(r.markdown,/does not by itself|not, by itself|do not by themselves/);
});
test('terminal acquisition can still fail the strict no-error workflow criterion',()=>{
  const input=fixture(),id=`${input.scenarios[0].id}-acquisition`,w=input.state.workflows[id];
  w.deterministic.complete=false;w.deterministic.errors=[{code:'incomplete_work',detail:'Premature terminal attempt before later completion'}];
  const r=screenReport(input),acq=r.completion.acquisitions.find(x=>x.caseId===input.scenarios[0].id);
  assert.equal(acq.status,'completed');assert.equal(acq.terminalState,'complete');assert.equal(acq.success,false);
  assert.equal(r.status,'completed-with-workflow-failures');assert.match(r.markdown,/Premature terminal attempt/);
});
test('early interruption reports all unconstructed grading packets and pending audit selection',()=>{
  const input=fixture();input.state={workflows:{},results:{},reviewPackets:{},grades:{}};
  const r=screenReport({...input,stopReason:'User interruption'});
  assert.equal(r.status,'stopped');assert.equal(r.dispatchCycleStatus,'stopped');
  assert.equal(r.grading.planned,24);assert.equal(r.grading.missingPackets,24);assert.equal(r.completion.pendingGradePackets,24);
  assert.equal(r.completion.pendingResultRecords,18);assert.equal(r.completion.pendingOutcomeWorkflows,24);
  assert.equal(r.humanAudit.status,'selection-pending');assert.deepEqual(r.humanAudit.selection,[]);
});
test('running workflow with a stale result cannot appear as a strict pass',()=>{
  const input=fixture(),id=`${input.scenarios[0].id}-ordinary`;input.state.workflows[id].status='running';
  const r=screenReport(input);assert.equal(r.status,'incomplete');assert.equal(r.dispatchCycleStatus,'incomplete');
  assert.equal(r.workflowOutcomes.find(w=>w.id===id).strictSuccess,null);assert.equal(r.completion.strictPassedWorkflows,23);
});
test('planned grading distinguishes absent records, invalid output and uncertain semantic review',()=>{
  const input=fixture(),ids=Object.keys(input.state.reviewPackets);
  delete input.state.reviewPackets[ids[0]];delete input.state.grades[ids[0]];
  delete input.state.grades[ids[1]];
  input.state.grades[ids[2]]={status:'pending',grade:null,errors:['Invalid reviewer pointer']};
  input.state.grades[ids[3]].grade.hUnits[0].status='uncertain';
  const g=gradingCoverage(input.scenarios,input.state);
  assert.equal(g.missingPackets,1);assert.equal(g.missingGradeRecords,1);assert.equal(g.invalidGrades,1);assert.equal(g.pendingSemanticGrades,1);
  assert.equal(g.missingGrades,2);assert.equal(g.pendingGradePackets,4);assert.equal(g.reviewedGrades,20);
  assert.equal(g.reviewedGrades+g.pendingGradePackets,g.planned);assert.equal(screenReport(input).status,'incomplete');
});
test('saved pending grade is pending even though its record exists',()=>{
  const input=fixture(),id=Object.keys(input.state.grades)[0];input.state.grades[id]={status:'pending',grade:null,errors:['schema failure']};
  const r=screenReport(input);assert.equal(r.completion.pendingGradePackets,1);assert.equal(r.grading.invalidGrades,1);assert.notEqual(r.status,'completed');
});
test('report interpretation leaves strict component ratings and the invocation ledger unchanged',()=>{
  const input=fixture(),before=structuredClone(input),r=screenReport(input);
  assert.deepEqual(input,before);assert.deepEqual(r.usage,usageReport(input.calls));
  assert.equal(r.usage.total.reportedTokens,200);assert.equal(r.usage.total.failedCalls,1);
  assert.equal(r.modes.ordinary.perLevel.demanding.components.H.rating,100);
  assert.equal(r.modes.prompted.perLevel.demanding.components.W.rating,100);
});

function comparisonFixture() {
  const input=fixture(),original=structuredClone(input.state),arms=['stock-maintained-notes','stock-history','loomtracer-native'];
  input.manifest.profile='comparison';input.manifest.arms=arms;
  input.state={workflows:{},results:{},reviewPackets:{},grades:{}};
  for(const [id,workflow] of Object.entries(original.workflows)) {
    if(workflow.control!=='primary') {input.state.workflows[id]=workflow;if(original.results[id])input.state.results[id]=original.results[id];continue;}
    for(const arm of arms) {
      const nextId=id.replace(workflow.caseId,`${workflow.caseId}-${arm}`);
      input.state.workflows[nextId]={...structuredClone(workflow),id:nextId,arm};
      input.state.results[nextId]={...structuredClone(original.results[id]),id:nextId,arm};
    }
  }
  for(const [id,result] of Object.entries(original.results))if(result.kind==='withheld-pair')input.state.results[id]=result;
  for(const scenario of input.scenarios) {
    const kinds=[...Array(8).fill('H'),...Array(7).fill('action'),'withheld'];
    for(const [index,kind] of kinds.entries()) {
      const blindId=`${scenario.id}-comparison-review-${index}`;
      input.state.reviewPackets[blindId]={caseId:scenario.id,packet:{blindId,kind}};
      input.state.grades[blindId]={status:'model-reviewed',errors:[],grade:{blindId,hUnits:[pass('fixture')],uncertainties:[],unsupportedClaims:[]}};
    }
  }
  const meta={status:'completed',evaluationConditionsHash:'same-evaluation',requestedModel:'same-model',returnedModel:null,reasoning:'medium',cliVersion:'same-cli',measurementScope:'functionality-v5-cli-reported-invocations-1'};
  input.calls=[];
  for(const scenario of input.scenarios) for(const arm of arms) {
    const tokenCount=arm==='stock-maintained-notes'?5:arm==='stock-history'?20:10;
    const usage={input_tokens:tokenCount-1,output_tokens:1,total_tokens:tokenCount,cached_input_tokens:0,reasoning_output_tokens:0};
    const checkpointGroup=`${scenario.id}:${arm}`;
    if(arm!=='stock-history')for(let episode=1;episode<=4;episode++)input.calls.push({...meta,id:`${checkpointGroup}-maintenance-${episode}`,arm,caseId:scenario.id,checkpointGroup,episode,stage:'maintenance',usage});
    for(const suffix of ['H','ordinary','prompted']) {
      const trialId=`${scenario.id}-${arm}-${suffix}`;
      input.calls.push({...meta,id:`${trialId}-call`,trialId,checkpointGroup,arm,caseId:scenario.id,stage:suffix==='H'?'reasoning':'action',usage});
    }
  }
  input.calls.push({...meta,id:'shared-control-cost',stage:'oracle',usage:{total_tokens:100000}});
  return input;
}

test('three-arm report counts acquisition and controls once and compares matched arm costs',()=>{
  const input=comparisonFixture(),before=structuredClone(input),r=screenReport(input);
  assert.deepEqual(input,before);
  assert.equal(r.profile,'comparison');assert.equal(r.completion.plannedWorkflows,42);
  assert.equal(r.completion.strictPassedWorkflows,42);assert.equal(r.completion.plannedResultRecords,36);
  assert.equal(r.grading.planned,48);assert.equal(r.grading.reviewedGrades,48);
  assert.equal(r.humanAudit.minimumPackets,12);assert.equal(r.status,'completed');
  for(const arm of input.manifest.arms)assert.equal(r.arms[arm].modes.ordinary.perLevel.demanding.components.H.rating,100);
  const notes=r.overheadComparisons['stock-maintained-notes'],history=r.overheadComparisons['stock-history'];
  assert.equal(notes.status,'available');assert.equal(notes.differences.ratio,2);
  assert.equal(notes.candidate.reportedTokens,210);assert.equal(notes.baseline.reportedTokens,105);
  assert.equal(history.baseline.reportedTokens,180);assert.equal(history.differences.difference,30);
  assert.equal(history.exploratory,true);assert.equal(history.equivalentSuccessfulWork,false);
  assert.equal(r.usage.total.reportedTokens,100495);assert.match(r.markdown,/stock-history/);
});

test('comparison cannot offer definitive overhead for incomplete native work or unknown usage',()=>{
  const input=comparisonFixture(),native=input.calls.find(c=>c.arm==='loomtracer-native'&&c.stage==='action');
  input.state.workflows[native.trialId].status='operation-limit';
  let report=screenReport(input);
  for(const comparison of Object.values(report.overheadComparisons)) {
    assert.equal(comparison.status,'unavailable');assert.equal(comparison.differences,null);
    assert.ok(comparison.errors.includes('loomtracer-native:incomplete-workload'));
  }
  input.state.workflows[native.trialId].status='completed';native.usage=null;
  report=screenReport(input);
  assert.equal(report.usage.total.unknownUsageCalls,1);
  for(const comparison of Object.values(report.overheadComparisons))assert.ok(comparison.errors.includes('loomtracer-native:unknown-usage'));
});

test('unresolved global semantic uncertainty suppresses definitive headline ratings without changing judgments or usage',()=>{
  const input=fixture(),id=Object.keys(input.state.grades)[0];
  input.state.grades[id].grade.uncertainties=['The required role is not clearly established by the source.'];
  const before=structuredClone(input),report=screenReport(input);
  assert.equal(report.status,'incomplete');assert.equal(report.grading.pendingSemanticGrades,1);
  assert.equal(report.completion.strictPassedWorkflows,23);
  assert.equal(report.workflowOutcomes.find(w=>w.id===`${input.scenarios[0].id}-H`).strictSuccess,null);
  for(const mode of ['ordinary','prompted']) {
    const band=report.modes[mode].perLevel.demanding;
    assert.equal(band.components.H.rating,null);assert.equal(band.components.H.provisionalRating,100);
    assert.equal(band.overall.rating,null);assert.equal(band.overall.semanticStatus,'pending');
    assert.ok(report.modes[mode].semanticReview.packetIds.includes(id));
    for(const row of report.modes[mode].cases) {
      assert.equal(row.semanticStatus,'pending');assert.ok(row.pendingPacketIds.includes(id));
      for(const component of ['H','Q','W']) {assert.equal(row[component].rating,null);assert.equal(row[component].provisionalRating,100);}
      assert.equal(row.D.status,'pending');assert.equal(row.D.provisionalStatus,'pass');
      assert.equal(row.H.completeChain,'pending');assert.equal(row.H.provisionalCompleteChain,'pass');
      assert.equal(row.completeTask,'pending');assert.equal(row.provisionalCompleteTask,'pass');
      assert.equal(row.H.counts.pass,row.H.counts.planned);
    }
  }
  assert.match(report.markdown,/provisional pending/);assert.deepEqual(input,before);
  assert.deepEqual(report.usage,usageReport(input.calls));
});


test('provisional case displays preserve known failures and original unit judgments',()=>{
  const input=fixture(),id=Object.keys(input.state.grades)[0],scenario=input.scenarios[1];
  input.state.grades[id].grade.uncertainties=['An unresolved issue requires review.'];
  const h=input.state.results[`${scenario.id}-H`];h.grade.hUnits[0].status='fail';
  const action=input.state.results[`${scenario.id}-ordinary`];action.grade.identity.status='fail';action.grade.completion.status='fail';
  const before=structuredClone(input),report=screenReport(input),row=report.modes.ordinary.cases.find(c=>c.id===scenario.id);
  assert.equal(row.H.completeChain,'fail');assert.equal(row.H.provisionalCompleteChain,'fail');
  assert.equal(row.H.counts.fail,1);assert.equal(row.D.status,'fail');assert.equal(row.D.provisionalStatus,'fail');
  assert.equal(row.completeTask,'fail');assert.equal(row.provisionalCompleteTask,'fail');
  assert.equal(row.H.semanticStatus,'pending');assert.deepEqual(input,before);
  assert.deepEqual(report.usage,usageReport(input.calls));
});
