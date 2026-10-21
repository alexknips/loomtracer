import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, writeFile, rm } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { buildCorpus, selectedScreenIds } from './corpus.mjs';
import { validAcquisitionPlan } from './world.mjs';
import { executeScreen, tasksFor } from './run-screen.mjs';
import { prepareScreen } from './freeze.mjs';
import { inspectNativeExecutable } from './native.mjs';
import { SOURCE_HERE, REVIEWED_FILES, armsForProfile, snapshotImplementation, validateAuthorization } from './package-support.mjs';
import { saveImmutable, sha256, loadState } from './artifacts.mjs';
import { PIN } from './runtime-execution.mjs';

function reviewResponse(packet) {
  const judgment=id=>({id,status:'fail',completeUnambiguous:false,supported:false,materialContradiction:false,
    reason:'Mock reviewer checks the complete trace and assigns failure to this deliberately non-solving fixture.',answerEvidence:[],sourceIds:[],absenceReviewed:true});
  const response={blindId:packet.blindId,hUnits:(packet.sourceTruth.hUnits??[]).map(u=>judgment(u.id)),qUnits:(packet.sourceTruth.qUnits??[]).map(u=>judgment(u.id)),
    wUnits:(packet.sourceTruth.wUnits??[]).map(u=>judgment(u.id)),controlUnits:(packet.sourceTruth.controlUnits??[]).map(u=>judgment(u.id)),
    identity:packet.kind==='action'?{...judgment('identity'),observed:false,late:false,category:'unobserved'}:null,
    completion:packet.kind==='action'?{status:'fail',reason:'The fixture deliberately does not complete the action task.'}:null,unsupportedClaims:[],uncertainties:[]};
  const complete=response.hUnits.find(u=>u.id==='fixture-complete');
  if(complete)Object.assign(complete,{status:'pass',completeUnambiguous:true,supported:true,answerEvidence:[{pointer:'/participant/answer/text',quote:'Mira approved the pilot only; production was excluded.'}],sourceIds:['fixture-source'],absenceReviewed:false});
  return response;
}

test('profiles preserve notes identifiers and require explicit comparison authorization',()=>{
  const s=buildCorpus()[0],notes=tasksFor(s),comparison=tasksFor(s,'comparison');
  assert.equal(notes.length,7);assert.equal(notes[0].id,`${s.id}-H`);
  assert.equal(comparison.length,13);assert.equal(comparison.filter(t=>t.control==='primary').length,9);
  assert.equal(new Set(comparison.map(t=>t.id)).size,13);
  assert.deepEqual(armsForProfile(),['stock-maintained-notes']);
  const approval={operator:'Offline test',approved:true,purpose:'Mock only',approvedAt:'2026-10-21T00:00:00Z',budgetTokens:100000};
  assert.throws(()=>validateAuthorization(approval,{profile:'comparison'}),/profile/);
  assert.equal(validateAuthorization({...approval,profile:'comparison'}).arms.length,3);
});

const executable=process.env.HIVEMIND_BENCHMARK_BINARY;
test('real native database with fully mocked three-arm provider completes all controller stages once', {skip:!executable,timeout:240000}, async t=>{
  const temp=await mkdtemp(join(tmpdir(),'v5-comparison-controller-'));
  try {
    const nativeConfig=await inspectNativeExecutable({executable:resolve(executable)});
    const corpus=buildCorpus(),selected=corpus.filter(s=>selectedScreenIds.includes(s.id));
    const offline=join(temp,'offline');await mkdir(offline);
    const files=Object.fromEntries(await Promise.all(REVIEWED_FILES.map(async name=>[name,sha256(await readFile(join(SOURCE_HERE,name)))])));
    const review={status:'passed',independent:true,reviewer:'Offline harness mock',providerCalls:0,summary:'This is a test fixture, not approval for live expenditure.',files};
    const implementationFiles=await snapshotImplementation(offline);
    const validation={passed:true,providerInvocations:0,review,selectedCaseIds:selectedScreenIds,corpusHash:sha256(corpus),implementationFiles,nativeValidation:{status:'passed',...nativeConfig,skippedTests:0}};
    await saveImmutable(offline,'validation.json',validation);await saveImmutable(offline,'validation.sha256.json',{sha256:sha256(validation)});await saveImmutable(offline,'corpus.controller.json',corpus);
    const authorizationPath=join(temp,'authorization.json');
    await writeFile(authorizationPath,JSON.stringify({operator:'Offline harness test',approved:true,purpose:'Mocked provider only',approvedAt:'2026-10-21T00:00:00Z',budgetTokens:100000,profile:'comparison'}));
    const root=join(temp,'run');
    await prepareScreen({root,offlineRoot:offline,authorizationPath,profile:'comparison',nativeBinary:executable,testOnly:true});
    let invocations=0;const maintenancePrompts=[],nativePrompts=[],gradedPackets=[];
    const invoke=async({prompt})=>{
      invocations++;let answer;
      if(prompt.includes('The calibration token is '))answer={ack:/The calibration token is ([A-Za-z0-9_-]+)\./.exec(prompt)[1]};
      else if(prompt.includes('fresh isolation calibration'))answer={token:null};
      else if(prompt.includes('\nPACKET\n')) {const packet=JSON.parse(prompt.split('\nPACKET\n')[1]);gradedPackets.push(packet);answer=reviewResponse(packet);}
      else if(prompt.startsWith('Maintain useful handoff notes')) {maintenancePrompts.push(prompt);answer={edits:[{op:'write',path:'notes.md',text:'# Mock notes\nDeliberately no answer is retained.'}]};}
      else if(prompt.startsWith('Maintain the provided synthetic reasoning history')) {nativePrompts.push(prompt);answer={operations:[]};}
      else {
        const payload=JSON.parse(prompt.slice(prompt.indexOf('\n\n')+2));
        if(payload.contract?.phase==='acquisition') {
          const scenario=selected.find(s=>s.acquisition.task===payload.task);assert.ok(scenario);
          answer=validAcquisitionPlan(scenario)[payload.brokerTrace.length];assert.ok(answer);
        } else if(prompt.includes('"kind":"recall"')&&!payload.brokerTrace.length) answer={op:'lookup',kind:'recall',query:'bounded trial',limit:3};
        else if(payload.questions)answer={op:'answer',answers:[],uncertainty:['PROBE_ONLY_SENTINEL']};
        else {const used=payload.brokerTrace.length;answer={actions:Array.from({length:16-used},()=>({op:'message',text:'Deliberately incomplete offline action fixture.'}))};}
      }
      return {status:'completed',answer:JSON.stringify(answer),requested_model:PIN.model,returned_model:null,cli_version:PIN.cli,reasoning:PIN.reasoning,processing:PIN.processing,
        usage:{input_tokens:8,output_tokens:2,total_tokens:10,cached_input_tokens:0,reasoning_output_tokens:0},events:[],violations:[]};
    };
    const outcome=await executeScreen({root,execute:true,invoke,skipCaffeinate:true,minStartIntervalMs:0});
    assert.equal(outcome.stopReason,null);assert.equal(outcome.tokens,10*invocations);
    t.diagnostic(`${invocations} mocked provider calls, ${outcome.tokens} synthetic reported tokens, actual Rust HiveMind checkpoints; zero real provider calls.`);
    const saved=await loadState(root),state=saved.value,report=JSON.parse(await readFile(join(root,outcome.reportPath),'utf8'));
    assert.equal(report.completion.plannedWorkflows,42);assert.equal(report.completion.endedDispatches,42);
    assert.equal(report.completion.plannedResultRecords,36);assert.equal(Object.keys(state.results).length,36);
    assert.equal(report.grading.planned,48);assert.equal(report.grading.reviewedGrades,48);
    assert.equal(maintenancePrompts.length,12);assert.equal(nativePrompts.length,12);
    assert.ok([...maintenancePrompts,...nativePrompts].every(p=>!p.includes('PROBE_ONLY_SENTINEL')));
    assert.equal(Object.keys(state.memory).length,36);
    assert.equal(report.usage.stages.maintenance.calls,24);
    assert.equal(report.usage.arms['stock-history'].calls,9);
    assert.equal(report.usage.arms['stock-history'].reportedTokens,90);
    assert.equal(new Set(report.usage.total.callIds).size,invocations);
    assert.equal(report.usage.total.calls,invocations);assert.equal(report.usage.total.unknownUsageCalls,0);
    assert.ok(Object.values(report.overheadComparisons).every(c=>c.status==='unavailable'&&c.errors.some(e=>e.includes('incomplete-workload'))));
    for(const packet of gradedPackets) for(const arm of armsForProfile('comparison'))assert.ok(!JSON.stringify(packet).includes(arm),'Blinded packet must omit arm identity');
    for(const s of selected)assert.equal(Object.values(state.workflows).filter(w=>w.caseId===s.id&&w.kind==='acquisition').length,1);
    for(const memory of Object.values(state.memory).filter(m=>m.format==='native'))assert.ok(memory.db.sha256);
  } finally {await rm(temp,{recursive:true,force:true});}
});
