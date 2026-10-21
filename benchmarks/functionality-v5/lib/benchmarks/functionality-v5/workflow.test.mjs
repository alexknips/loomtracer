import test from 'node:test';
import assert from 'node:assert/strict';
import { buildCorpus, selectedScreenIds, bindAcquisition } from './corpus.mjs';
import { createWorld, applyAction, validAcquisitionPlan, validActionPlan } from './world.mjs';
import { performWorkflow, taskPrompt } from './workflow.mjs';
import { controlSources } from './run-screen.mjs';
import { applyMemoryEdits } from './memory.mjs';

const selected=buildCorpus().filter(s=>selectedScreenIds.includes(s.id));
const fakeCall=answer=>({status:'completed',usage:{input_tokens:100,output_tokens:100,total_tokens:200},answer:JSON.stringify(answer)});
function acquired(s) {
  const world=createWorld(s,{phase:'acquisition'});
  for(const action of validAcquisitionPlan(s)) applyAction(world,action);
  return world;
}
function ready(s,world,a,pending) {
  const d=s.worldDefinition;
  if(a.op==='reuse'||a.op==='recognize'||a.op==='message')return true;
  if(a.op==='ask')return !a.fields.includes(d.conditional?.field)||world.questionAnswers[d.conditional.dependsOnField]!==undefined;
  if(a.op==='run') {
    const op=[...d.acquisitionChecks,d.original,...d.fresh].find(x=>x.operation===a.operation);
    return (op.prerequisites??[]).every(id=>world.available[id])&&(op.fields??[]).every(field=>Object.hasOwn(d.knownFields,field)||world.questionAnswers[field]!==undefined);
  }
  return ['finish','block'].includes(a.op)&&pending.length===1;
}

for(const original of selected) test(`oracle and withheld workflows fit exact cumulative delivery: ${original.family}`,async()=>{
  const acquisition=acquired(original);
  assert.equal(acquisition.status,'complete');
  const summary='A concise originating handoff with actual observations and explicit scope. '.repeat(25);
  const raw=JSON.stringify({op:'finish',summary,artifacts:Object.keys(acquisition.artifacts)});
  const s=bindAcquisition(original,acquisition,{summary,traceSources:[{id:'origin-trace-1',actor:'Originating agent',conversation:'originating-work',text:raw}]}).scenario;
  for(const control of ['oracle','withheld']) {
    const task={id:`test-${original.family}-${control}`,kind:'action',mode:'ordinary',control};
    const sources=controlSources(s,task);
    let pending=validActionPlan(s,{withheld:control==='withheld'}).filter(a=>a.op!=='lookup');
    const planner=createWorld(s,{phase:'action',acquisition,withheld:control==='withheld',eligibleSourceIds:sources.map(x=>x.id),visibleArtifactIds:Object.keys(acquisition.artifacts)});
    let calls=0;
    const obtain=async()=>{
      const batch=pending.filter(a=>ready(s,planner,a,pending));
      assert.ok(batch.length,'Offline valid witness makes progress');
      const before=structuredClone(planner);
      for(const a of batch)applyAction(planner,a,{batchStart:before,eligibleSourceIds:sources.map(x=>x.id),visibleArtifactIds:Object.keys(acquisition.artifacts)});
      pending=pending.filter(a=>!batch.includes(a));calls++;
      return {value:{actions:batch},call:fakeCall({actions:batch}),parseError:null};
    };
    const writes=new Map();
    const result=await performWorkflow({scenario:s,task,acquisition,sources,obtain,save:async(n,v)=>writes.set(n,structuredClone(v))});
    assert.ok(['completed','blocked'].includes(result.status),JSON.stringify({status:result.status,errors:result.errors,world:result.world.errors}));
    assert.ok(result.view.historicalBytes<=32768);
    assert.ok(result.operationsUsed<=16);
    assert.ok(calls<=6);
    const prior=calls;
    const restored=await performWorkflow({scenario:s,task,acquisition,sources,obtain,save:async()=>{},restore:result});
    assert.equal(calls,prior,'Completed workflow is never redispatched');
    assert.deepEqual(restored,result);
  }
});

test('originating agent receives only episode one and not later identity/answer key',async()=>{
  const s=selected[0],task={id:'acq-visible',kind:'acquisition',mode:'ordinary',control:'shared'};
  let captured;
  await performWorkflow({scenario:s,task,sources:s.episodes[0].sources,save:async()=>{},obtain:async(id,prompt)=>{
    captured=prompt;return {value:{op:'finish',summary:'Incomplete originating attempt.',artifacts:[]},call:fakeCall({}),parseError:null};
  }});
  assert.match(captured,/staging-only/);
  assert.doesNotMatch(captured,/authoritative ticket bridge maps/);
  assert.doesNotMatch(captured,/h-premises|q-necessary|"identityEvidence"|authoritative ticket bridge maps/);
});

test('ordinary versus prompted action instructions remain distinct without hidden output requirements',()=>{
  const s=selected[0],base={scenario:s,world:null,history:[],trace:[],operationsUsed:0};
  const ordinary=taskPrompt({...base,task:{kind:'action',mode:'ordinary'}});
  const prompted=taskPrompt({...base,task:{kind:'action',mode:'prompted'}});
  assert.doesNotMatch(ordinary,/Before substantive work, explicitly assess/);
  assert.match(prompted,/Before substantive work, explicitly assess/);
  assert.match(ordinary,/8192 UTF-8 bytes of its serialized participant payload/);
  assert.match(ordinary,/32768 bytes of distinct historical payload/);
  assert.match(ordinary,/Every actual replay remains recorded in cumulative delivery bytes and CLI-reported token usage/);
  assert.doesNotMatch(ordinary,/hUnits|qUnits|wUnits|identityEvidence/);
});

test('invalid provider status cannot execute partial operations',async()=>{
  const s=selected[0];
  await assert.rejects(performWorkflow({scenario:s,task:{id:'partial',kind:'acquisition',mode:'ordinary',control:'shared'},sources:s.episodes[0].sources,
    save:async()=>{},obtain:async()=>({value:{op:'run',operation:s.worldDefinition.acquisitionChecks[0].operation},call:{status:'failed',usage:null},parseError:null})}),/failed or usage unknown/);
});

test('source controls do not advertise unavailable notes lookup operations',()=>{
  const scenario=selected[0];
  for(const control of ['oracle','withheld']) {
    const prompt=taskPrompt({scenario,task:{kind:'H',mode:'prompted',control},world:null,history:[],trace:[],operationsUsed:0});
    assert.match(prompt,/complete eligible original-source evidence/);
    assert.match(prompt,/No notes search\/open or archive search is available/);
    assert.doesNotMatch(prompt,/"kind":"search"|"kind":"open"/);
    assert.match(prompt,/32768 bytes of active historical context/);
    assert.doesNotMatch(prompt,/hUnits|identityEvidence/);
  }
});

test('maintained-note fork must deliver references before using historical artifacts',async()=>{
  const original=selected[0],acquisition=acquired(original),s=bindAcquisition(original,acquisition).scenario;
  const memory=applyMemoryEdits(null,{edits:[{op:'write',path:'notes.md',text:'# Empty handoff\nNo artifact was retained.'}]},{eligibleSources:s.episodes.flatMap(e=>e.sources),revision:'r1'});
  const task={id:'no-evidence',kind:'action',mode:'ordinary',control:'primary'};
  const result=await performWorkflow({scenario:s,task,acquisition,memory,sources:s.episodes.flatMap(e=>e.sources),save:async()=>{},obtain:async()=>{
    const value={op:'reuse',artifactId:s.worldDefinition.reuse[0].artifactId};return {value,call:fakeCall(value),parseError:null};
  }});
  assert.equal(result.status,'operation-limit');
  assert.equal(result.world.reused.length,0);
  assert.ok(result.trace.every(r=>!r.receipt.ok));
});
