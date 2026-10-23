import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { inspectNativeExecutable } from './native.mjs';
import { applyMaintenanceForArm, createMemoryView, prepareNativeView, lookupForView, accountHistoricalDelivery, validateCitation, replayPackets } from './memory.mjs';

const binary=process.env.HIVEMIND_BENCHMARK_BINARY;
test('real native packets unlock only delivered retained refs, cite exact native responses, and isolate forks', {skip:!binary?'Actual Rust CLI required for native delivery proof':false,timeout:30000},async t=>{
  const root=await mkdtemp(join(tmpdir(),'v5-native-delivery-'));t.after(()=>rm(root,{recursive:true,force:true}));
  const nativeConfig=await inspectNativeExecutable({executable:binary});
  const sources=[{id:'s1',order:1,actor:'Ada',text:'Ada authorizes a staging relay pilot only; queue evidence supports it.'},{id:'s2',order:2,actor:'Ben',text:'Future production withdrawal evidence.'}];
  const memory=await applyMaintenanceForArm('loomtracer-native',null,{operations:[{tool:'capture_decision',key:'pilot',sourceIds:['s1'],args:{
    title:'Staging relay pilot',rationale:'Queue wait evidence supports a controlled staging relay pilot with no production adoption.',topic_keys:['relay'],
    options:[{label:'Staging pilot'}],chosen_option_label:'Staging pilot',decided_by:'human:Ada',
    grounding:[{kind:'evidence',content:'Queue wait supports a bounded test.',source:'s1'}],binds:{level:'something',statement:'Staging only.'},undo_cost:{level:'cheap'}
  }}]},{eligibleSources:sources,sourceCutoff:1,revision:'opaque-r1',nativeConfig,checkpointRoot:join(root,'checkpoint')});
  const view=createMemoryView(memory,sources,{mode:'native'}), other=createMemoryView(memory,sources,{mode:'native'});
  await prepareNativeView(view,{nativeConfig,taskRoot:join(root,'task')});await prepareNativeView(other,{nativeConfig,taskRoot:join(root,'other')});
  const record=memory.records.find(r=>r.kind==='decision');
  await assert.rejects(()=>lookupForView(view,{op:'expand',id:record.id}),/already delivered/);
  await assert.rejects(()=>lookupForView(view,{op:'source',id:'s1'}),/already delivered/);
  const packet=await lookupForView(view,{op:'recall',query:'staging relay pilot',limit:1});
  assert.ok(packet.evidence[0].recordIds.includes(record.id));
  assert.equal(packet.evidence[0].text,JSON.stringify(packet.payload.native));
  assert.equal(validateCitation(view,packet.payload.citation).valid,false);
  await assert.rejects(()=>lookupForView(view,{op:'source',id:'s1'}),/already delivered/);
  accountHistoricalDelivery(view,JSON.stringify([packet.payload]),{id:'delivered',packets:[packet]});
  assert.equal(validateCitation(view,packet.payload.citation).valid,true);
  assert.equal(validateCitation(other,packet.payload.citation).valid,false);
  assert.equal(validateCitation(view,'source:s1').valid,false);
  const source=await lookupForView(view,{op:'source',id:'s1'});
  accountHistoricalDelivery(view,JSON.stringify([source.payload]),{id:'source',packets:[source]});
  assert.equal(validateCitation(view,'source:s1').valid,true);
  await assert.rejects(()=>lookupForView(view,{op:'source',id:'s2'}),/already delivered/);
  assert.ok((await lookupForView(view,{op:'expand',id:record.id})).payload.native);
  const tampered=structuredClone(packet);tampered.payload.native.native={fabricated:'source:s2'};
  assert.throws(()=>replayPackets(view,[tampered]),/integrity/);
  assert.deepEqual(other.eligibleSourceIds,[]);
  const restored=JSON.parse(JSON.stringify(view));
  await prepareNativeView(restored,{nativeConfig,taskRoot:join(root,'task')});
  assert.equal(validateCitation(restored,packet.payload.citation).valid,true);
  assert.ok(replayPackets(restored,[packet,source]).length===2);
  assert.ok((await lookupForView(restored,{op:'expand',id:record.id})).payload.native);
  await assert.rejects(()=>prepareNativeView(restored,{nativeConfig,taskRoot:join(root,'other')}),/fork identity/);

});
