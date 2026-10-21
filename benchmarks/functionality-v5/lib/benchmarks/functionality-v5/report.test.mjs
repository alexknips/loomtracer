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
  assert.equal(r.version,REPORT_INTERPRETATION_VERSION);assert.equal(r.evaluationReportVersion,'functionality-v5-report-1.0.2');
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
  assert.equal(r.version,'functionality-v5-report-1.0.2');
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
