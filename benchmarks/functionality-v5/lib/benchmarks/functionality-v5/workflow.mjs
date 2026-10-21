import { createWorld, applyAction, publicWorldContract } from './world.mjs';
import { createMemoryView, lookup, accountHistoricalDelivery, validateCitation, replayPackets,
  migrateLegacyMemoryView, REPLAY_POLICY_ID } from './memory.mjs';
import { sha256 } from './artifacts.mjs';

export const WORKFLOW_VERSION = 'functionality-v5-workflow-1.1.0';
const clone = structuredClone;
const bytes = value => Buffer.byteLength(typeof value === 'string' ? value : JSON.stringify(value));
const isDone = world => world && !['running','active'].includes(world.status);
const cutoff = sources => Math.max(0,...sources.map((s,i)=>s.order??s.sourceOrder??i+1));

export function taskPrompt({scenario,task,world,history,trace,operationsUsed}) {
  const acquisition = task.kind === 'acquisition', reasoning = task.kind === 'H';
  const taskText = acquisition ? scenario.acquisition.task : reasoning ?
    'Reconstruct the expressed reasoning and its current applicability. Answer every question completely and unambiguously. Distinguish unestablished facts from established negatives; do not invent missing reasons or actions.' : scenario.task;
  const instructions = [
    'You are acting in a bounded synthetic workspace. Use only the provided task, historical delivery, and broker receipts. Return one JSON object. Broker operations are requested through JSON, not external tools.',
    'Return {"actions":[...]} or one operation. A batch may contain independent operations only; results become available on the next invocation. Dependent operations must wait for their prerequisite receipts.',
    `This fork permits 16 broker operations total; ${operationsUsed} have been used. Every operation, including lookup and finish, counts.${acquisition?' Newly revealed originating sources are provided below.':' At most 32768 bytes of distinct historical payload may be exposed, and each invocation may contain at most 32768 bytes of active historical context.'}`,
  ];
  if (!acquisition) {
    const sourceControl=['oracle','withheld'].includes(task.control);
    instructions.push(sourceControl ?
      'This diagnostic view already provides its complete eligible original-source evidence. No notes search/open or archive search is available. Assertions not established by this evidence remain unresolved. Citations: source:<source-id>.' :
      'Memory access: {"op":"lookup","kind":"search","query":"..."}, {"op":"lookup","kind":"open","path":"...","startLine":1,"endLine":40}, or {"op":"lookup","kind":"source","id":"..."}. Each lookup response may deliver at most 8192 UTF-8 bytes of its serialized participant payload, including reference metadata. Controller-only audit evidence is excluded. Source expansion is restricted to eligible references already delivered. Citations: source:<source-id> or note:<revision>:<path>:L<first>-L<last>.');
    instructions.push('Exact serialized historical payloads are deduplicated; overlapping nonidentical passages count separately toward distinct exposure. All retained requested payloads are replayed without controller summaries or answer-based selection. Every actual replay remains recorded in cumulative delivery bytes and CLI-reported token usage; cumulative replay bytes are not a stopping threshold. Historical artifact references must be established in delivered retained evidence before reuse; current tool names alone do not grant access to historical work. Identity-dependent execution requires delivered retained evidence for its scoped mapping/authority, but no particular outward statement format is required.');
  }
  if (reasoning) instructions.push('Final operation: {"op":"answer","answers":[{"questionId":"...","text":"...","citations":["..."]}],"uncertainty":[]}. Give concise stated explanations, not private internal deliberation.');
  else if (task.mode === 'prompted') instructions.push('Before substantive work, explicitly assess whether this is the same underlying prior question/problem, a distinct one, or unresolved, with scoped supporting evidence. Use valid prior work, identify necessary clarifications, and explain the resulting action choices.');
  const payload = {
    task: taskText,
    ...(reasoning ? {questions:scenario.questions} : {contract:publicWorldContract(scenario,acquisition?'acquisition':'action')}),
    historicalDelivery:history,
    brokerTrace:trace,
  };
  return `${instructions.join('\n')}\n\n${JSON.stringify(payload)}`;
}

/** Explicit child-state migration only; callers must preserve the parent artifact. */
export function migrateDeliveryLimitedWorkflow(original) {
  if (original?.status !== 'delivery-limit' || !Number.isSafeInteger(original.operationsUsed) || original.operationsUsed >= 16 || original.operationsUsed < 0) throw new Error('Only an unfinished delivery-limit workflow below 16 operations may migrate');
  if (!Array.isArray(original.callIds) || !Array.isArray(original.trace) || !Array.isArray(original.packets) || !original.view) throw new Error('Saved workflow checkpoint is incomplete');
  if (original.world && !['active','running'].includes(original.world.status)) throw new Error('Terminal world cannot be reopened');
  if (original.trace.length !== original.operationsUsed || new Set(original.callIds).size !== original.callIds.length) throw new Error('Saved workflow operation or call ledger differs');
  if (JSON.stringify(original.view.deliveries.map(row=>row.id)) !== JSON.stringify(original.callIds) || JSON.stringify((original.deliveries??[]).map(row=>row.id)) !== JSON.stringify(original.callIds) || original.callIds.some((id,index)=>id!==`${original.id}-step-${index+1}`)) throw new Error('Saved workflow dispatch and delivery ledgers differ');
  const migrated = clone(original);
  migrated.view = migrateLegacyMemoryView(original.view, original.packets);
  // Validate that unchanged pending deliveries fit the new active/distinct caps.
  // Keep their original packet list in this checkpoint; normal replay deduplicates it.
  replayPackets(migrated.view, migrated.packets);
  migrated.status = 'running'; migrated.replayPolicyId = REPLAY_POLICY_ID;
  migrated.replayMigration = { fromStateHash: sha256(original), fromStatus: original.status,
    toPolicy: REPLAY_POLICY_ID, preservedCallCount: original.callIds.length,
    preservedOperationCount: original.operationsUsed, preservedCumulativeBytes: original.view.historicalBytes };
  return migrated;
}

/** A replayable workflow. Only obtain() can dispatch a model invocation. */
export async function performWorkflow({scenario,task,acquisition=null,memory=null,sources=[],obtain,save,restore=null,onProgress=()=>{}}) {
  let state = restore ? clone(restore) : {
    id:task.id,caseId:scenario.id,kind:task.kind,mode:task.mode,control:task.control??'primary',replayPolicyId:REPLAY_POLICY_ID,
    status:'running',world:task.kind==='H'?null:createWorld(scenario,{phase:task.kind==='acquisition'?'acquisition':'action',acquisition,withheld:task.control==='withheld'}),
    view:task.kind==='acquisition'?null:createMemoryView(memory,sources,{mode:task.control==='primary'?'notes':task.control,withheldIds:task.control==='withheld'?scenario.withheldSourceIds:[],sourceCutoff:cutoff(sources)}),
    packets:task.kind==='acquisition'?[{payload:{sources}}]:[],trace:[],callIds:[],operationsUsed:0,answer:null,citations:[],errors:[],deliveries:[],
  };
  if (state.status !== 'running') return state;
  if (state.id !== task.id || state.caseId !== scenario.id) throw new Error('Restored workflow identity differs from the requested task');
  if (!state.packets.length && state.view?.initialPacket) state.packets.push(state.view.initialPacket);
  while(state.status==='running' && state.operationsUsed<16) {
    const id=`${task.id}-step-${state.callIds.length+1}`;
    let historical, historicalText;
    try {
      if(state.view) state.packets=replayPackets(state.view,state.packets);
      historical=state.packets.map(p=>p.payload??p);
      historicalText=JSON.stringify(historical);
      if(state.view) accountHistoricalDelivery(state.view,historicalText,{id,packets:state.packets});
    } catch(error) { state.status='delivery-limit';state.errors.push(error.message);break; }
    const prompt=taskPrompt({scenario,task,world:state.world,history:historical,trace:state.trace,operationsUsed:state.operationsUsed});
    const delivery={id,historicalBytes:bytes(historicalText),promptBytes:bytes(prompt),promptHash:sha256(prompt),packetHashes:state.packets.map(p=>p.sha256??sha256(p)),sourceCutoff:state.view?.frozen.sourceCutoff??cutoff(sources),memoryRevision:state.view?.frozen.revision?.id??memory?.id??null,memoryHash:state.view?.frozen.revision?.sha256??memory?.sha256??null,replayPolicyId:REPLAY_POLICY_ID,uniqueExposureBytes:state.view?.uniqueExposureBytes??null,cumulativeHistoricalBytes:state.view?.historicalBytes??null};
    await save(`deliveries/${id}.json`,{...delivery,historicalDelivery:historical});
    const stage=task.kind==='acquisition'?'acquisition':task.control==='oracle'?'oracle':task.control==='withheld'?'withheld':task.kind==='H'?'reasoning':'action';
    const r=await obtain(id,prompt,{stage,arm:task.control==='primary'?'stock-maintained-notes':null,caseId:scenario.id,scenarioId:scenario.id,kind:task.kind,mode:task.mode,control:task.control,workflowId:task.id,step:state.callIds.length+1});
    state.callIds.push(id);state.deliveries.push(delivery);
    if(r.call.status!=='completed' || !r.call.usage) throw new Error(`Invocation failed or usage unknown: ${id}`);
    if(r.parseError || !r.value || typeof r.value!=='object' || Array.isArray(r.value)) {
      state.status='invalid-output';state.errors.push(r.parseError??'JSON object required');state.answer=r.call.answer;
    } else {
      const actions=r.value.actions??[r.value];
      if(!Array.isArray(actions)||!actions.length) {state.status='invalid-output';state.errors.push('Nonempty actions array required');}
      else {
        // A batch never silently exceeds the public cap or executes after completion.
        const batchStart=state.world?clone(state.world):null;
        for(const action of actions) {
          if(state.status!=='running') {state.errors.push('Operation emitted after terminal operation');break;}
          if(state.operationsUsed>=16) break;
          state.operationsUsed++;
          let receipt;
          try {
            if(action?.op==='lookup' && state.view) {
              const packet=lookup(state.view,{...action,op:action.kind??action.operation});
              const updated=replayPackets(state.view,[...state.packets,packet]);
              const retained=updated.find(p=>sha256(p.payload)===sha256(packet.payload));
              receipt={ok:true,packetHash:retained.sha256,exactRepeat:updated.length===state.packets.length,delivery:'Requested passage will be in historicalDelivery on the next invocation.'};
              state.packets=updated;
            } else if(action?.op==='answer' && task.kind==='H') {
              if(!Array.isArray(action.answers)||!action.answers.every(a=>typeof a.questionId==='string'&&typeof a.text==='string'&&Array.isArray(a.citations??[]))) throw new Error('Invalid answer shape');
              state.answer=clone(action);state.status='completed';
              for(const answer of action.answers) for(const citation of answer.citations??[]) state.citations.push({citation,...validateCitation(state.view,citation)});
              receipt={ok:true,terminal:true};
            } else if(state.world) {
              const visibleArtifactIds=task.kind==='acquisition'?null:Object.keys(state.world.priorArtifacts??{}).filter(id=>state.view.deliveredEvidence.some(e=>e.text.includes(id)));
              receipt=applyAction(state.world,action,{batchStart,eligibleSourceIds:state.view?.eligibleSourceIds??[],visibleArtifactIds});
              if(isDone(state.world)) {state.status=state.world.status==='complete'?'completed':state.world.status;state.answer=clone(state.world.result??state.world.terminal??action);}
            } else throw new Error('Operation not available in this fork');
          } catch(error) {receipt={ok:false,error:error.message};}
          if(state.view && task.kind!=='H') {
            for(const citation of [...(action?.citations??[]),...(action?.evidence??[])]) state.citations.push({citation,...validateCitation(state.view,typeof citation==='string'&&!citation.startsWith('source:')&&!citation.startsWith('note:')?`source:${citation}`:citation)});
          }
          state.trace.push({operation:clone(action),receipt:clone(receipt),index:state.operationsUsed});
        }
      }
    }
    await save(`workflow-states/${task.id}/step-${state.callIds.length}.json`,state);
    onProgress({kind:'workflow-step',id:task.id,status:state.status,operations:state.operationsUsed});
  }
  if(state.status==='running') state.status='operation-limit';
  await save(`workflows/${task.id}.json`,state);
  return state;
}
