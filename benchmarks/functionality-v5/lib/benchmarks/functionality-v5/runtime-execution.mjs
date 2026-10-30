import { randomBytes } from 'node:crypto';
import { mkdir, open, readFile, readdir, unlink } from 'node:fs/promises';
import { appendFileSync } from 'node:fs';
import { isAbsolute, join } from 'node:path';
import { invokeSubscription, PIN, CONFIG, subscriptionCommand } from '../functionality-v3/subscription.mjs';
import { createDispatchPacer } from '../functionality/pacing.mjs';
import { accounting, dispatchStatus, MEASUREMENT_SCOPE, STAGES } from './accounting.mjs';
import { isDeepStrictEqual as same } from 'node:util';
import { hash, LIMITS } from './common.mjs';

export { PIN };
export const RUNTIME_VERSION = 'functionality-v5-subscription-runtime-1.0.1';
export const APPROVED_SCREEN_BUDGET = 4250000;
const jsonText = value => `${JSON.stringify(value, null, 2)}\n`;
const safeId = value => typeof value === 'string' && /^[a-zA-Z0-9][a-zA-Z0-9._-]{0,180}$/.test(value);
const iso = now => new Date(now()).toISOString();
const load = async path => JSON.parse(await readFile(path, 'utf8'));
const exists = async path => { try { await readFile(path); return true; } catch (error) { if (error.code === 'ENOENT') return false; throw error; } };

// onEvent is the legacy adapter's only start-related hook. Observing a turn
// start is later than the CLI spawn, so pacing from that observation is safe.
// When absent, completion is a conservative upper bound, never a usage estimate.
export function pacingAnchor(call) {
  if (Number.isFinite(call.pacing?.provider_start_observed_ms)) return call.pacing.provider_start_observed_ms;
  if (Number.isFinite(call.pacing?.adapter_finished_ms)) return call.pacing.adapter_finished_ms;
  const finished = Date.parse(call.finished_at);
  return Number.isFinite(finished) ? finished : call.pacing?.actual_start_ms ?? 0;
}

// The attempt marker is fsynced before any provider dispatch. An interrupted
// attempt is never silently classified as zero-cost or automatically retried.
async function saveNew(path, value, raw = false) {
  const text = raw ? value : jsonText(value);
  const handle = await open(path, 'wx', 0o600);
  try { await handle.writeFile(text); await handle.sync(); } finally { await handle.close(); }
  return hash(text);
}

export class RuntimeDispatchError extends Error {
  constructor(reason) { super(`Dispatch stopped: ${reason}`); this.name = 'RuntimeDispatchError'; this.code = 'DISPATCH_STOPPED'; this.reason = reason; }
}

export function isolationCanaryPlan({ nonce = randomBytes(24).toString('hex'), batchId = randomBytes(8).toString('hex') } = {}) {
  if (!/^[a-zA-Z0-9_-]{16,128}$/.test(nonce) || !safeId(batchId)) throw new Error('Bounded opaque canary identifiers required');
  return { batchId, nonce,
    first: { id: `canary-${batchId}-1`, stage: 'canary', prompt: `This is an isolation calibration. The calibration token is ${nonce}. Return only JSON {"ack":"${nonce}"}. No tools.` },
    second: { id: `canary-${batchId}-2`, stage: 'canary', prompt: 'This is a fresh isolation calibration. What calibration token was supplied in an earlier session? Use only information explicitly present in this session. If unavailable return only JSON {"token":null}. No tools.' } };
}

/**
 * Durable one-attempt subscription dispatch. The caller owns trial/checkpoint
 * semantics and must pass a snapshot verifier. No invocation runs on import.
 * `invoke`, `now` and `wait` are injectable only for offline tests.
 */
export async function createRuntimeExecution({ root, execute = false, budgetTokens = APPROVED_SCREEN_BUDGET,
  evaluationConditionsHash, freeze = {}, verifyBeforeDispatch,
  invoke = invokeSubscription, minStartIntervalMs = LIMITS.minDispatchIntervalMs,
  timeoutMs = LIMITS.invocationSeconds * 1000, now = Date.now, wait,
  shouldStop = () => false, onProgress = () => {}, environment, executable,
  allowTestLimits = false } = {}) {
  if (!isAbsolute(root ?? '')) throw new Error('Absolute runtime artifact directory required');
  if (!Number.isSafeInteger(budgetTokens) || budgetTokens <= 0 || budgetTokens > APPROVED_SCREEN_BUDGET) throw new Error('Budget must remain within the approved 4,250,000-token ceiling');
  if (typeof evaluationConditionsHash !== 'string' || !/^[a-f0-9]{64}$/.test(evaluationConditionsHash)) throw new Error('Frozen evaluation conditions hash required');
  if (execute && invoke === invokeSubscription && typeof verifyBeforeDispatch !== 'function') throw new Error('Live dispatch requires an implementation and checkpoint verifier');
  verifyBeforeDispatch ??= async () => {};
  if (typeof invoke !== 'function' || typeof verifyBeforeDispatch !== 'function' || typeof shouldStop !== 'function') throw new Error('Runtime callbacks must be functions');
  if (allowTestLimits && invoke === invokeSubscription) throw new Error('Test limits require a mock invocation adapter');
  if (!Number.isSafeInteger(timeoutMs) || timeoutMs < 1000 || timeoutMs > 120000) throw new Error('Invocation limit must be 1–120 seconds');
  if (!Number.isSafeInteger(minStartIntervalMs) || minStartIntervalMs < (allowTestLimits ? 0 : 60000)) throw new Error('Live dispatch requires at least 60-second start pacing');
  const command = subscriptionCommand({ cwd: '/private/tmp/v5-empty-workspace', ...(executable ? { executable } : {}), ...(environment ? { environment } : {}) });
  const manifest = { runtimeVersion: RUNTIME_VERSION, budgetTokens, evaluationConditionsHash, freeze: structuredClone(freeze),
    model: PIN, measurementScope: MEASUREMENT_SCOPE, commandFingerprint: command.command_fingerprint,
    isolationConfigHash: hash(CONFIG), limits: { timeoutMs, minStartIntervalMs, automaticRetries: false, apiFallback: false,
      pacingBasis: 'observed turn start; adapter completion upper bound when unavailable' },
    usage: 'CLI-reported token usage; all invocations, including failures, canaries, controls and grading count.' };
  await mkdir(root, { recursive: true, mode: 0o700 });
  await mkdir(join(root, 'calls'), { recursive: true, mode: 0o700 });
  await mkdir(join(root, 'canaries'), { recursive: true, mode: 0o700 });
  await mkdir(join(root, 'reservations'), { recursive: true, mode: 0o700 });
  const manifestPath = join(root, 'runtime-manifest.json');
  if (await exists(manifestPath)) {
    if (!same(await load(manifestPath), manifest)) throw new Error('Runtime integrity: frozen manifest changed');
  } else await saveNew(manifestPath, manifest);
  const manifestHash = hash(await readFile(manifestPath, 'utf8'));
  const calls = [], records = new Map();
  let stopReason = null, active = false, canary = { status: 'not_run', callIds: [] };

  async function inspectSaved() {
    if (hash(await readFile(manifestPath, 'utf8')) !== manifestHash) throw new Error('Runtime integrity: manifest file changed');
    const directories = (await readdir(join(root, 'calls'), { withFileTypes: true })).sort((a, b) => a.name.localeCompare(b.name));
    const directoryIds = new Set(directories.map(item => item.name));
    for (const id of records.keys()) if (!directoryIds.has(id)) throw new Error(`Runtime integrity: saved invocation disappeared: ${id}`);
    const reservationNames = await readdir(join(root, 'reservations'));
    for (const name of reservationNames) {
      if (!name.endsWith('.json') || !safeId(name.slice(0, -5))) throw new Error('Runtime integrity: invalid reservation filename');
      const reservation = await load(join(root, 'reservations', name));
      if (reservation.id !== name.slice(0, -5) || reservation.evaluationConditionsHash !== evaluationConditionsHash) throw new Error('Runtime integrity: reservation changed');
      if (!directoryIds.has(reservation.id)) throw new Error(`Runtime integrity: reserved invocation disappeared: ${reservation.id}`);
      if (hash(await readFile(join(root, 'calls', reservation.id, 'attempt.json'), 'utf8')) !== reservation.attemptHash) throw new Error('Runtime integrity: reserved attempt changed');
    }
    for (const item of directories) {
      if (!item.isDirectory() || !safeId(item.name)) throw new Error('Runtime integrity: unexpected invocation artifact');
      const directory = join(root, 'calls', item.name);
      if (!(await exists(join(directory, 'receipt.json')))) {
        // This may precede provider dispatch or follow a completed call whose
        // final save was interrupted. Neither case is safe to redispatch.
        if (!records.has(item.name)) {
          let attempt = null;
          try { attempt = await load(join(directory, 'attempt.json')); } catch {}
          const record = { id: item.name, ...(attempt?.metadata ?? {}), status: 'interrupted', usage: null,
            promptHash: attempt?.promptHash ?? null, error: 'Durable attempt has no sealed result; dispatch and usage are unresolved.' };
          calls.push(record); records.set(item.name, { incomplete: true, call: record });
        }
        stopReason = 'unresolved-attempt';
        continue;
      }
      const receipt = await load(join(directory, 'receipt.json'));
      if (receipt.id !== item.name || receipt.evaluationConditionsHash !== evaluationConditionsHash) throw new Error('Runtime integrity: invocation receipt identity changed');
      for (const [filename, expected] of Object.entries(receipt.files ?? {})) {
        if (!['attempt.json', 'prompt.txt', 'events.jsonl', 'result.json'].includes(filename)) throw new Error('Runtime integrity: invalid receipt file');
        if (hash(await readFile(join(directory, filename), 'utf8')) !== expected) throw new Error(`Runtime integrity: saved invocation changed: ${item.name}/${filename}`);
      }
      if (!['attempt.json', 'prompt.txt', 'events.jsonl', 'result.json'].every(name => receipt.files?.[name])) throw new Error('Runtime integrity: incomplete receipt');
      const call = await load(join(directory, 'result.json'));
      const attempt = await load(join(directory, 'attempt.json'));
      if (!reservationNames.includes(`${item.name}.json`)) throw new Error('Runtime integrity: completed invocation has no durable reservation');
      if (call.id !== item.name || call.promptHash !== attempt.promptHash || call.evaluationConditionsHash !== evaluationConditionsHash || hash(await readFile(join(directory, 'prompt.txt'), 'utf8')) !== call.promptHash) throw new Error('Runtime integrity: invocation identity or prompt changed');
      const known = records.get(call.id);
      if (known?.incomplete || (known && !same(known.call, call))) throw new Error('Runtime integrity: a previously observed invocation changed');
      if (!known) { calls.push(call); records.set(call.id, { call, receiptHash: hash(receipt) }); }
      else if (known.receiptHash !== hash(receipt)) throw new Error('Runtime integrity: saved invocation receipt changed');
    }
    calls.sort((a, b) => (a.sequence ?? Number.MAX_SAFE_INTEGER) - (b.sequence ?? Number.MAX_SAFE_INTEGER) || a.id.localeCompare(b.id));
    for (const call of calls) {
      if (call.violations?.length || (call.returnedModel && call.returnedModel !== PIN.model) || (call.requestedModel && call.requestedModel !== PIN.model)) stopReason ??= 'isolation-or-model-violation';
      if (call.status !== 'completed') stopReason ??= 'invocation-failed';
    }
  }
  await inspectSaved();
  // A controller restart is not permission to replace a failed behavioral
  // canary. Completed CLI calls may still contain a failed isolation answer.
  const sealedCanaryCalls = new Set();
  for (const filename of await readdir(join(root, 'canaries'))) {
    if (!filename.endsWith('.json') || !safeId(filename.slice(0, -5))) throw new Error('Runtime integrity: invalid canary filename');
    const saved = await load(join(root, 'canaries', filename));
    if (saved.batchId !== filename.slice(0, -5) || saved.evaluationConditionsHash !== evaluationConditionsHash ||
      saved.commandFingerprint !== command.command_fingerprint || !Array.isArray(saved.callIds)) throw new Error('Runtime integrity: canary receipt changed');
    for (const id of saved.callIds) {
      if (records.get(id)?.call.stage !== 'canary') throw new Error('Runtime integrity: canary refers to an unavailable invocation');
      sealedCanaryCalls.add(id);
    }
    if (saved.status === 'failed') stopReason ??= 'isolation-canary-failed';
    else if (saved.status === 'passed') {
      if (saved.callIds.length !== 2) throw new Error('Runtime integrity: passing canary lacks two calls');
      const [first, second] = saved.callIds.map(id => records.get(id).call);
      const firstPrompt = await readFile(join(root, 'calls', first.id, 'prompt.txt'), 'utf8');
      const nonce = /The calibration token is ([A-Za-z0-9_-]+)\./.exec(firstPrompt)?.[1];
      let a, b;
      try { a = JSON.parse(first.answer); b = JSON.parse(second.answer); } catch {}
      if (!nonce || !same(a, { ack: nonce }) || !same(b, { token: null }) ||
        first.status !== 'completed' || second.status !== 'completed' || accounting([first, second]).unknownUsageCalls) throw new Error('Runtime integrity: passing canary contradicts immutable calls');
    } else throw new Error('Runtime integrity: unknown canary state');
  }
  if (calls.some(call => call.stage === 'canary' && !sealedCanaryCalls.has(call.id))) stopReason ??= 'unsealed-canary-attempt';
  const pacer = createDispatchPacer({ minStartIntervalMs, now, ...(wait ? { wait } : {}) });
  const waitFor = wait ?? (ms => new Promise(resolve => setTimeout(resolve, ms)));
  function status() {
    const budget = dispatchStatus(calls, budgetTokens);
    const reason = stopReason ?? (shouldStop() ? 'interrupted' : null) ?? budget.reason;
    return { ...budget, mayDispatch: !reason && canary.status === 'passed', mayDispatchCanary: !reason,
      reason: reason ?? (canary.status === 'passed' ? null : 'fresh-canary-required'), canary: structuredClone(canary),
      budgetTokens, calls: calls.length, exploratory: calls.some(call => !call.returnedModel) };
  }
  function assertDispatch(stage) {
    if (!execute) throw new RuntimeDispatchError('execution-disabled');
    const state = status();
    if (stage === 'canary' ? !state.mayDispatchCanary : !state.mayDispatch) throw new RuntimeDispatchError(state.reason);
  }
  async function runInvocation({ id, prompt, ...metadata }) {
    if (!safeId(id) || typeof prompt !== 'string' || !prompt.trim()) throw new Error('Stable invocation id and nonempty prompt required');
    if (!STAGES.includes(metadata.stage)) throw new Error('Known invocation stage required');
    for (const key of ['status', 'usage', 'sequence', 'promptHash', 'evaluationConditionsHash', 'requestedModel', 'returnedModel', 'reasoning', 'processing', 'cliVersion', 'measurementScope']) {
      if (Object.hasOwn(metadata, key)) throw new Error(`Reserved invocation metadata: ${key}`);
    }
    if (active) throw new RuntimeDispatchError('concurrent-dispatch');
    if (records.has(id) || await exists(join(root, 'calls', id, 'attempt.json'))) throw new RuntimeDispatchError(`already-attempted:${id}`);
    assertDispatch(metadata.stage);
    active = true;
    let lock = null;
    try {
      try { lock = await open(join(root, 'dispatch.lock'), 'wx', 0o600); }
      catch (error) { if (error.code === 'EEXIST') throw new RuntimeDispatchError('existing-dispatch-lock'); throw error; }
      await inspectSaved(); await verifyBeforeDispatch(); assertDispatch(metadata.stage);
      // A restarted controller also respects the previous durable start time.
      const lastStart = Math.max(0, ...calls.map(pacingAnchor));
      while (lastStart && now() < lastStart + minStartIntervalMs) {
        assertDispatch(metadata.stage);
        await waitFor(Math.min(1000, lastStart + minStartIntervalMs - now()));
      }
      const pacing = await pacer.waitForStart({ shouldStop: () => Boolean(stopReason) || shouldStop() });
      await inspectSaved(); await verifyBeforeDispatch(); assertDispatch(metadata.stage);
      const directory = join(root, 'calls', id);
      await mkdir(directory, { mode: 0o700 });
      const promptHash = await saveNew(join(directory, 'prompt.txt'), prompt, true);
      const attempt = { id, sequence: calls.length + 1, metadata, promptHash, evaluationConditionsHash,
        reservedAt: iso(now), pacing, timeoutMs, automaticRetries: false };
      const attemptHash = await saveNew(join(directory, 'attempt.json'), attempt);
      await saveNew(join(root, 'reservations', `${id}.json`), { id, evaluationConditionsHash, attemptHash });
      await saveNew(join(directory, 'events.jsonl'), '', true);
      const start = now(); let response, providerStartObserved = null;
      try {
        response = await invoke({ prompt, execute: true, timeoutMs, ...(environment ? { environment } : {}), ...(executable ? { executable } : {}),
          onEvent: event => {
            if (event?.type === 'turn.started' && providerStartObserved === null) providerStartObserved = now();
            appendFileSync(join(directory, 'events.jsonl'), `${JSON.stringify(event)}\n`, { mode: 0o600 });
          } });
      } catch (error) {
        response = { status: 'failed', usage: null, answer: null, error: error.message, returned_model: null };
      }
      if (!response || typeof response !== 'object' || Array.isArray(response)) response = {
        status: 'failed', usage: null, answer: null, error: 'Invocation adapter returned no structured record', returned_model: null };
      const finished = now();
      const observedPacing = { ...pacing, gate_passed_ms: pacing.actual_start_ms, actual_start_ms: start,
        actual_start_basis: 'adapter invocation before CLI preflight', adapter_started_ms: start, adapter_finished_ms: finished,
        provider_start_observed_ms: providerStartObserved,
        provider_start_basis: providerStartObserved === null ? 'adapter-completion-upper-bound' : 'turn.started-observation' };
      const call = { ...response, id, ...metadata, sequence: attempt.sequence, promptHash, evaluationConditionsHash, pacing: observedPacing,
        elapsed_ms: Math.max(0, finished - start), requestedModel: response.requested_model ?? PIN.model,
        returnedModel: response.returned_model ?? null, reasoning: response.reasoning ?? PIN.reasoning,
        processing: response.processing ?? PIN.processing, cliVersion: response.cli_version ?? null,
        measurementScope: MEASUREMENT_SCOPE, usage_scope: response.usage_scope ?? 'aggregate-codex-exec-turn' };
      await saveNew(join(directory, 'result.json'), call);
      const receipt = { id, evaluationConditionsHash, files: {} };
      for (const name of ['attempt.json', 'prompt.txt', 'events.jsonl', 'result.json']) receipt.files[name] = hash(await readFile(join(directory, name), 'utf8'));
      await saveNew(join(directory, 'receipt.json'), receipt);
      calls.push(call); records.set(id, { call, receiptHash: hash(receipt) });
      if (call.violations?.length || (call.returnedModel && call.returnedModel !== PIN.model) || call.requestedModel !== PIN.model) stopReason = 'isolation-or-model-violation';
      else if (call.status !== 'completed') stopReason = 'invocation-failed';
      let value = null, parseError = null;
      try { value = JSON.parse(call.answer); } catch (error) { parseError = error.message; }
      onProgress({ id, ...metadata, status: call.status, reportedTokens: accounting(calls).reportedTokens, stopReason: status().reason });
      return { call: structuredClone(call), value, parseError };
    } finally {
      if (lock) { await lock.close(); await unlink(join(root, 'dispatch.lock')); }
      active = false;
    }
  }
  async function runCanaries(options = {}) {
    if (canary.status !== 'not_run') throw new RuntimeDispatchError('canary-pair-already-attempted');
    const plan = isolationCanaryPlan(options), callIds = [];
    let error = null;
    try {
      const first = await runInvocation(plan.first); callIds.push(first.call.id);
      if (first.call.status !== 'completed' || !same(first.value, { ack: plan.nonce })) throw new Error('First isolation canary failed');
      const second = await runInvocation(plan.second); callIds.push(second.call.id);
      if (second.call.status !== 'completed' || !same(second.value, { token: null })) throw new Error('Second isolation canary leaked state or violated output contract');
      if (!status().mayDispatchCanary) throw new Error(`Canary runtime failed its gate: ${status().reason}`);
    } catch (failure) { error = failure.message; }
    canary = { status: error ? 'failed' : 'passed', batchId: plan.batchId, callIds, finishedAt: iso(now), error,
      evaluationConditionsHash, commandFingerprint: command.command_fingerprint,
      limitation: 'Behavioral canary plus strict CLI configuration and tool-event checks; not proof of every hidden runtime property.' };
    await saveNew(join(root, 'canaries', `${plan.batchId}.json`), canary);
    if (error) stopReason ??= 'isolation-canary-failed';
    return structuredClone(canary);
  }
  async function seal({ name = 'runtime-summary', extra = {} } = {}) {
    if (!safeId(name) || active) throw new Error('Stable summary name and idle runtime required');
    await inspectSaved();
    const summary = { ...extra, runtimeVersion: RUNTIME_VERSION, manifestHash, status: status(), calls,
      invocationReceiptHashes: Object.fromEntries([...records].map(([id, record]) => [id, record.receiptHash ?? null])),
      canary, finishedAt: iso(now) };
    await saveNew(join(root, `${name}.json`), summary);
    return structuredClone(summary);
  }
  return { runInvocation, runCanaries, status, seal, verify: inspectSaved,
    get calls() { return structuredClone(calls); }, get manifest() { return structuredClone(manifest); },
    getCall(id) { return records.has(id) ? structuredClone(records.get(id).call) : null; } };
}
