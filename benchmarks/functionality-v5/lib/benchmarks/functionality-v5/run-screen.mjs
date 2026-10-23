#!/usr/bin/env node
import { spawn } from 'node:child_process';
import { realpathSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { parseArgs } from 'node:util';
import { check } from './common.mjs';
import { createRuntimeExecution } from './runtime-execution.mjs';
import { bindAcquisition, eligibleSources } from './corpus.mjs';
import { deterministicChecks } from './world.mjs';
import { maintenancePromptForArm, applyMaintenanceForArm } from './memory.mjs';
import { armsForProfile, primaryTasksFor, sharedTasksFor } from './package-support.mjs';
import { performWorkflow } from './workflow.mjs';
import { prepareScreen, verifyScreen } from './freeze.mjs';
import { saveImmutable, parse, loadState, saveState, sha256 } from './artifacts.mjs';
import { buildHPackets, buildActionPacket, buildWithheldPacket, gradePrompt, validateGrade, mergeHGrades, qualificationFixture, qualifyGrader, humanAuditSelection } from './grading.mjs';
import { screenReport } from './report.mjs';

export function tasksFor(scenario, profile = 'notes') { return [...primaryTasksFor(scenario, profile), ...sharedTasksFor(scenario)]; }

/** Frozen evidence selection for controls, fixed before any originating run.
 * Action controls need current facts, authority and original artifact receipts;
 * raw originating deliberation is retained for H but is not replayed as action context.
 */
export function controlSources(scenario,task) {
  const candidates=eligibleSources(scenario,{withheld:task.control==='withheld'}).filter(s=>!s.id.startsWith('noise-'));
  if(task.kind==='H') return candidates;
  const ids=new Set(['src-0001','src-0003','src-0004','src-0006','src-0007','src-0010','src-0011','src-0012']);
  for(const item of scenario.worldDefinition.reuse) ids.add(`acq-${item.operation}`);
  return candidates.filter(s=>ids.has(s.id));
}

export async function executeScreen({root,execute=false,invoke,skipCaffeinate=false,minStartIntervalMs=60000,shouldStop=()=>false,onProgress=()=>{},canariesOnly=false}={}) {
  check(execute,'Explicit execution required');
  check(Boolean(invoke) || process.platform === 'darwin', 'Live execution requires macOS, the pinned Codex CLI and caffeinate; other platforms are offline-only');
  const manifest=await verifyScreen(root), corpus=await parse(join(root,'corpus.controller.json'));
  const profile=manifest.profile??'notes', arms=armsForProfile(profile);
  check(!manifest.testOnly||invoke,'Offline-only manifest cannot dispatch live');
  check(!skipCaffeinate||invoke,'Live screen requires owned caffeinate');
  let saved=await loadState(root),state=structuredClone(saved.value),stopReason=null,caffeinate=null;
  for(const key of ['workflows','bound','memory','results','reviewPackets','grades']) state[key]??={};
  state.errors??=[];
  const persist=async()=>{saved=await saveState(root,saved,state);};
  const save=async(name,value)=>{
    const digest=await saveImmutable(root,name,value);
    if(name.startsWith('workflow-states/')) {state.workflows[value.id]=value;await persist();}
    return digest;
  };
  const runtime=await createRuntimeExecution({root:join(root,'execution'),execute:true,budgetTokens:manifest.budgetTokens,
    evaluationConditionsHash:manifest.evaluationConditionsHash,freeze:{manifestHash:sha256(manifest)},verifyBeforeDispatch:()=>verifyScreen(root),
    minStartIntervalMs,shouldStop,onProgress:r=>onProgress({kind:'invocation',...r}),...(invoke?{invoke,allowTestLimits:true}:{})});
  const obtain=async(id,prompt,meta)=>{
    if(shouldStop()) throw new Error('User interruption');
    const prior=runtime.getCall(id);
    if(prior) {
      check(prior.promptHash===sha256(prompt),`Continuation prompt changed: ${id}`);
      check(prior.status==='completed'&&prior.usage,`No retry of failed or unknown invocation: ${id}`);
      let value=null,parseError=null;try{value=JSON.parse(prior.answer);}catch(e){parseError=e.message;}
      return {call:prior,value,parseError};
    }
    const r=await runtime.runInvocation({id,prompt,...meta});
    check(r.call.status==='completed'&&r.call.usage,`Invocation failed or usage unknown: ${id}`);
    return r;
  };
  async function perform(s,task,acquisition,memory,sources) {
    if(state.workflows[task.id]?.status && state.workflows[task.id].status!=='running') return state.workflows[task.id];
    const result=await performWorkflow({scenario:s,task,acquisition,memory,sources,obtain,save,restore:state.workflows[task.id],onProgress,nativeConfig:manifest.nativeConfig,taskRoot:join(root,'native','tasks',task.id)});
    if(result.world) result.deterministic=deterministicChecks(s,result.world);
    state.workflows[task.id]=result;await persist();
    await save(`completed-workflows/${task.id}.json`,result);
    return result;
  }
  async function gradePacket(packet,scenario,workflow=null) {
    if(state.grades[packet.blindId])return state.grades[packet.blindId];
    await save(`review/${packet.blindId}.json`,packet);
    const forks=workflow?(Array.isArray(workflow)?workflow:[workflow]):[];
    await save(`review-private/${packet.blindId}.json`,{caseId:scenario.id,workflowIds:forks.map(w=>w.id),arms:forks.map(w=>w.arm??null),
      memoryCheckpoints:[...new Set(forks.filter(w=>w.control==='primary').flatMap(w=>scenario.episodes.map(e=>`checkpoints/${scenario.id}/${profile==='notes'?'':`${w.arm}/`}episode-${e.number}.json`)))],
      workflowArtifacts:forks.map(w=>`completed-workflows/${w.id}.json`),
      firstObservedLoss:'not established; inspect actual source, retained notes, delivered passages and answer before attributing loss'});
    await save(`review-stage-evidence/${packet.blindId}.json`,{blindId:packet.blindId,firstObservedLoss:{status:'pending',stage:'not established'},
      forks:forks.map(w=>({kind:w.kind,mode:w.mode,acquisitionOutcome:scenario.acquisitionOutcome,
        retainedRepresentation:w.view?.frozen.revision??null,memoryRevision:w.view?.frozen.revision?.id??null,
        deliveredEvidence:w.view?.deliveredEvidence??[],deliveryLedger:w.view?.deliveries??[],
        answer:w.answer,trace:w.trace,citations:w.citations}))});
    state.reviewPackets[packet.blindId]={packet,caseId:scenario.id,family:scenario.family,arm:forks[0]?.arm??null,control:forks[0]?.control??null,workflowIds:forks.map(w=>w.id)};await persist();
    const r=await obtain(`grade-${packet.blindId}`,gradePrompt(packet),{stage:'grading',caseId:scenario.id,scenarioId:scenario.id,blindId:packet.blindId});
    const parsed=validateGrade(packet,r.call.answer);
    state.grades[packet.blindId]=parsed;await save(`grades/${packet.blindId}.json`,parsed);await persist();
    // Contract errors are preserved and left pending, never repaired with hidden evidence.
    check(parsed.status==='model-reviewed',`Grading packet requires review: ${packet.blindId}: ${parsed.errors.join('; ')}`);
    return parsed;
  }
  async function runCase(original) {
    const acqTask={id:`${original.id}-acquisition`,kind:'acquisition',mode:'ordinary',control:'shared'};
    const acq=await perform(original,acqTask,null,null,original.episodes[0].sources);
    if(!state.bound[original.id]) {
      const traceSources=acq.callIds.map((id,i)=>({id:`origin-trace-${i+1}`,actor:'Originating agent',conversation:'originating-work',text:runtime.getCall(id).answer}));
      const bound=bindAcquisition(original,acq.world,{summary:acq.world?.terminal?.summary??acq.answer?.summary??'',traceSources});
      await save(`acquisition/${original.id}.json`,{...bound,workflowHash:sha256(acq),obligationTemplateHash:sha256({h:original.hUnits,q:original.qUnits,w:original.wUnits})});
      state.bound[original.id]=bound;await persist();
    }
    const bound=state.bound[original.id],s=bound.scenario;
    const memories={};
    for(const arm of arms) {
      let memory=null,seen=[];
      for(const episode of s.episodes) {
        seen.push(...episode.sources);
        const id=`${s.id}-${profile==='notes'?'':`${arm}-`}maintenance-${episode.number}`;
        if(state.memory[id]) {memory=state.memory[id];continue;}
        let response=null, invocation=null;
        if(arm!=='stock-history') {
          const prompt=maintenancePromptForArm(arm,memory,episode);
          const r=await obtain(id,prompt,{stage:'maintenance',arm,caseId:s.id,scenarioId:s.id,episode:episode.number,checkpointGroup:`${s.id}:${arm}`});
          check(!r.parseError,`Invalid ${arm} maintenance JSON: ${id}`);
          response=r.value;invocation=id;
        }
        memory=await applyMaintenanceForArm(arm,memory,response,{eligibleSources:seen,revision:profile==='notes'?`revision-${episode.number}`:`revision-${sha256([s.id,arm,episode.number]).slice(0,20)}`,sourceCutoff:Math.max(...seen.map(x=>x.order)),
          nativeConfig:manifest.nativeConfig,checkpointRoot:join(root,'native','checkpoints',s.id,`episode-${episode.number}`)});
        await save(`checkpoints/${s.id}/${profile==='notes'?'':`${arm}/`}episode-${episode.number}.json`,{arm,memory,sourceCutoff:memory.sourceCutoff,sourceHash:sha256(seen),implementationHash:manifest.implementationHash,invocation});
        state.memory[id]=memory;await persist();
        onProgress({kind:'maintenance',arm,caseId:s.id,episode:episode.number,invocation});
      }
      memories[arm]=memory;
    }
    const tasks=tasksFor(s,profile),executions=[];
    for(const task of tasks) {
      // Controls are shared once and receive only their predeclared original evidence.
      const sources=task.control==='primary'?eligibleSources(s):controlSources(s,task);
      executions.push(await perform(s,task,acq.world,task.control==='primary'?memories[task.arm]:null,sources));
    }
    for(const workflow of executions.filter(w=>w.control!=='withheld')) {
      if(state.results[workflow.id]) continue;
      const sources=eligibleSources(s).filter(x=>!x.id.startsWith('noise-'));
      let grade;
      if(workflow.kind==='H') {
        const packets=buildHPackets(s,workflow,sources),grades=[];
        for(const packet of packets) grades.push(await gradePacket(packet,s,workflow));
        grade=mergeHGrades(s,packets,grades);check(grade.status==='model-reviewed','H partitions require review');
      } else grade=await gradePacket(buildActionPacket(s,workflow,sources),s,workflow);
      const result={id:workflow.id,caseId:s.id,scenarioId:s.id,arm:workflow.control==='primary'?workflow.arm:null,
        kind:workflow.kind,mode:workflow.mode,control:workflow.control,status:workflow.status,grade:grade.grade,deterministic:workflow.deterministic??null,humanAudit:'pending'};
      state.results[workflow.id]=result;await save(`results/${workflow.id}.json`,result);await persist();
    }
    const withheld=executions.filter(w=>w.control==='withheld');
    const controlId=`${s.id}-withheld-control`;
    if(!state.results[controlId]) {
      const packet=buildWithheldPacket(s,withheld,eligibleSources(s,{withheld:true}).filter(x=>!x.id.startsWith('noise-')));
      const grade=await gradePacket(packet,s,withheld);
      state.results[controlId]={id:controlId,caseId:s.id,kind:'withheld-pair',control:'withheld',status:withheld.every(w=>['completed','blocked'].includes(w.status))?'completed':'incomplete',grade:grade.grade};
      await save(`results/${controlId}.json`,state.results[controlId]);await persist();
    }
    onProgress({kind:'case-complete',caseId:s.id,tokens:runtime.status().usage.reportedTokens});
  }
  try {
    if(!skipCaffeinate) {
      caffeinate=spawn('/usr/bin/caffeinate',['-i','-w',String(process.pid)],{stdio:'ignore'});
      await new Promise((yes,no)=>{caffeinate.once('spawn',yes);caffeinate.once('error',no);});
      await save(`caffeinate/start-${Date.now()}.json`,{pid:caffeinate.pid,ownerPid:process.pid,at:new Date().toISOString()});
    }
    const canary=await runtime.runCanaries();onProgress({kind:'canary',...canary});check(canary.status==='passed','Fresh isolation canary failed');
    if(!canariesOnly) {
      if(!state.graderQualification) {
        const fixture=qualificationFixture();await save('grader-qualification/packet.json',fixture.packet);
        const r=await obtain('grader-qualification',gradePrompt(fixture.packet),{stage:'grading',purpose:'fixed-grader-qualification'});
        state.graderQualification=qualifyGrader(fixture.packet,r.call.answer,fixture.expected);
        await save('grader-qualification/result.json',state.graderQualification);await persist();
      }
      check(state.graderQualification.passed,'Grader qualification failed');
      for(const s of corpus) await runCase(s);
      const packets=Object.values(state.reviewPackets).map(row=>row.packet);
      const families=Object.fromEntries(Object.values(state.reviewPackets).map(row=>[row.packet.blindId,`${row.family}:${row.arm??'shared'}`]));
      state.auditSelection=humanAuditSelection(packets,packets.map(p=>state.grades[p.blindId]),{families});await persist();
    }
  } catch(error) {
    stopReason=error.message;state.errors.push({at:new Date().toISOString(),reason:stopReason});await persist();onProgress({kind:'stopped',reason:stopReason});
  } finally {
    if(caffeinate) {const pid=caffeinate.pid;caffeinate.kill();await save(`caffeinate/stop-${Date.now()}.json`,{pid,ownerPid:process.pid,at:new Date().toISOString(),terminationRequested:true});}
  }
  await runtime.verify();
  const report=screenReport({manifest,scenarios:corpus,state,calls:runtime.calls,stopReason});
  const suffix=`${saved.sequence}-${runtime.calls.length}-${Date.now()}`;
  await save(`reports/report-${suffix}.json`,report);await save(`reports/report-${suffix}.md`,report.markdown);
  await runtime.seal({name:`runtime-summary-${suffix}`});
  return {root,status:report.status,stopReason,reportPath:`reports/report-${suffix}.json`,tokens:report.usage.total.reportedTokens,unknownUsage:report.usage.total.unknownUsageCalls,calls:runtime.calls.length};
}

export const HELP = `V5 memory comparison calibration screen

Usage:
  npm run screen -- --output /absolute/new/run --offline-root /absolute/offline \
    --authorization /absolute/operator-authorization.json [--profile notes|comparison] [--native-binary /absolute/hivemind] [--execute] [--canaries-only]

Preparation requires a provider-free offline freeze with a source-bound independent
authoring review and a NEW explicit operator authorization JSON containing:
  operator: name of authorizing operator
  approved: true
  purpose: purpose of this fresh run
  approvedAt: ISO timestamp
  budgetTokens: positive integer up to 4250000
  profile: notes (default) or comparison (must match --profile)

Without --execute, creates an immutable run only and makes NO provider calls.
With --execute, first runs a fresh two-call isolation canary and grader qualification;
only successful checks permit the three demanding cases. The notes profile uses
maintained notes only. The comparison profile uses maintained notes, stock history
and the native Rust HiveMind CLI, with shared acquisition and source controls.
Comparison requires --native-binary; its path, version and hash are frozen.
--canaries-only requires --execute and stops after the canary; its new run remains
a canary diagnostic rather than a complete screen and cannot be continued through
this CLI. Every call counts toward budget.

Live execution is deliberately pinned to macOS, ChatGPT subscription authentication,
Codex CLI 0.163.0-alpha.10.2 at the ChatGPT.app bundled path, gpt-6.1-sol, medium
reasoning and standard processing. Unsupported installations FAIL CLOSED. This
package does not change models, substitute API billing, retry failures or silently
weaken isolation. /usr/bin/caffeinate is owned and stopped by the controller.

Fresh output directories only. Existing directories are never overwritten. The
public package does not resume historical private runs or authorize a retry after
a failed/unknown-usage call. Sources, review, settings and authorization are frozen.
The budget is a subsequent-dispatch threshold, not a hard provider token cap.
`;
const main=()=>{try{return realpathSync(process.argv[1])===fileURLToPath(import.meta.url);}catch{return false;}};
if(main()) {
  const {values}=parseArgs({options:{output:{type:'string'},'offline-root':{type:'string'},authorization:{type:'string'},profile:{type:'string',default:'notes'},'native-binary':{type:'string'},execute:{type:'boolean',default:false},'canaries-only':{type:'boolean',default:false},help:{type:'boolean',short:'h'}}});
  if(values.help) console.log(HELP);
  else {
    check(values.output && values['offline-root'] && values.authorization, 'Fresh output, reviewed offline root and new operator authorization required; use --help');
    check(!values['canaries-only'] || values.execute, '--canaries-only requires --execute');
    const prepared=await prepareScreen({root:values.output,offlineRoot:values['offline-root'],authorizationPath:values.authorization,profile:values.profile,nativeBinary:values['native-binary']});
    const {root,manifest}=prepared;
    console.log(JSON.stringify({kind:'prepared',root,profile:manifest.profile,arms:manifest.arms,budgetTokens:manifest.budgetTokens,providerCalls:0}));
    if(values.execute) {
      const frozen=pathToFileURL(join(root,'snapshot/benchmarks/functionality-v5/run-screen.mjs')).href;
      const run=frozen===import.meta.url?executeScreen:(await import(frozen)).executeScreen;
      let stopped=false;process.on('SIGINT',()=>{stopped=true;});process.on('SIGTERM',()=>{stopped=true;});
      console.log(JSON.stringify(await run({root,execute:true,shouldStop:()=>stopped,canariesOnly:values['canaries-only'],onProgress:p=>console.log(JSON.stringify({...p,at:new Date().toISOString()}))})));
    }
  }
}
