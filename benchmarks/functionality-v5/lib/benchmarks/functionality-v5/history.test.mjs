import test from 'node:test';
import assert from 'node:assert/strict';
import { applyMaintenanceForArm, maintenancePromptForArm, createMemoryView, lookup, accountHistoricalDelivery, validateCitation } from './memory.mjs';
import { taskPrompt, performWorkflow } from './workflow.mjs';
import { buildCorpus } from './corpus.mjs';

const sources = [
  { id:'s1', order:1, actor:'Ada', conversation:'original', text:'Ada adopted relay for the controlled pilot only.' },
  { id:'s2', order:2, actor:'Ben', conversation:'noise', text:'Unrelated lunch discussion.' },
  { id:'s3', order:3, actor:'Ada', conversation:'later', text:'Future evidence must not leak.' },
];
const freeze = (previous=null, visible=sources, revision='r1', sourceCutoff=2) => applyMaintenanceForArm('stock-history',previous,null,{eligibleSources:visible,revision,sourceCutoff});
const deliver = view => accountHistoricalDelivery(view, JSON.stringify([view.initialPacket.payload]), {id:'first',packets:[view.initialPacket]});

test('history checkpoint preserves exact source occurrences including noise without model maintenance', async()=>{
  assert.throws(()=>maintenancePromptForArm('stock-history',null,sources),/no model maintenance/);
  const one = await freeze();
  assert.deepEqual(one.sources,sources.slice(0,2));
  const two=await freeze(one,sources,'r2',3);
  assert.equal(two.parentHash,one.sha256); assert.equal(one.sources.length,2);
  await assert.rejects(()=>applyMaintenanceForArm('stock-history',null,{edits:[]},{eligibleSources:sources,revision:'r3',sourceCutoff:3}),/curated/);
  await assert.rejects(()=>freeze(one,[{...sources[0],text:'Edited'},sources[1]],'r4'),/rewrite/);
});

test('history source citations and evidence visibility are per fork and cutoff',async()=>{
  const revision=await freeze(), a=createMemoryView(revision,sources,{mode:'history'}), b=createMemoryView(revision,sources,{mode:'history'});
  assert.equal(validateCitation(a,'source:s1').valid,false);deliver(a);
  assert.equal(validateCitation(a,'source:s1').valid,true);assert.equal(validateCitation(b,'source:s1').valid,false);
  assert.equal(validateCitation(a,'source:s3').valid,false);
  assert.throws(()=>lookup(a,{op:'search',query:'relay'}),/no hidden notes/);
  assert.throws(()=>lookup(a,{op:'source',id:'s3'}),/already delivered/);
  assert.throws(()=>createMemoryView(revision,[{...sources[0],text:'tamper'},sources[1]],{mode:'history'}),/immutable checkpoint/);
});

test('history overflow drops an oldest prefix only, records omissions, and never unlocks omitted source',async()=>{
  const many=Array.from({length:50},(_,i)=>({id:`s${i}`,order:i+1,text:`noise ${i} `+'x'.repeat(1000)}));
  const revision=await freeze(null,many,'long',50), view=createMemoryView(revision,many,{mode:'history'});
  const payload=view.initialPacket.payload, omitted=payload.omittedOldestMessages;
  assert.ok(omitted>0); assert.deepEqual(payload.sources,many.slice(omitted));
  assert.ok(Buffer.byteLength(JSON.stringify([payload]))<=32768);
  deliver(view); assert.equal(validateCitation(view,'source:s0').valid,false);
  assert.throws(()=>lookup(view,{op:'source',id:'s0'}),/already delivered/);
});

test('history and native prompts describe actual access while preserving ordinary D behavior',()=>{
  const scenario=buildCorpus()[0];
  for(const arm of ['stock-history','loomtracer-native']) {
    const prompt=taskPrompt({scenario,task:{kind:'action',mode:'ordinary',control:'primary',arm},world:null,history:[],trace:[],operationsUsed:0});
    assert.ok(!prompt.includes('Before substantive work, explicitly assess'));
    assert.ok(!prompt.includes('kind":"search"'));
    assert.match(prompt,arm==='stock-history'?/Chronological|chronologically/:/kind":"recall/);
  }
});
