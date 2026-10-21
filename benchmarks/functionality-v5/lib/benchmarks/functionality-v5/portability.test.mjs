import test from 'node:test';
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { PACKAGE_ROOT, SOURCE_HERE, REVIEWED_FILES, validateAuthorization, validateAuthoringReview, validateOutputPath } from './package-support.mjs';
import { sha256 } from './artifacts.mjs';
import { prepareScreen } from './freeze.mjs';

const approval = () => ({ operator: 'Test operator', approved: true, purpose: 'Offline test only', approvedAt: '2026-10-20T12:00:00Z', budgetTokens: 100000 });

test('package root and output rules are independent of the calling working directory', () => {
  assert.equal(SOURCE_HERE, join(PACKAGE_ROOT, 'lib/benchmarks/functionality-v5'));
  assert.equal(validateOutputPath(join(PACKAGE_ROOT, '.artifacts/test')), join(PACKAGE_ROOT, '.artifacts/test'));
  assert.throws(() => validateOutputPath(join(SOURCE_HERE, 'run')), /outside/);
  assert.throws(() => validateOutputPath('/tmp/offline/run', { offlineRoot: '/tmp/offline' }), /non-overlapping/);
});

test('fresh operator approval cannot be inherited or synthesized by the runner', async () => {
  assert.throws(() => validateAuthorization(undefined), /authorization/);
  for (const key of ['operator', 'approved', 'approvedAt', 'purpose', 'budgetTokens']) {
    const value = approval(); delete value[key]; assert.throws(() => validateAuthorization(value));
  }
  assert.throws(() => validateAuthorization({ ...approval(), budgetTokens: 4250001 }), /budget/);
  assert.throws(() => validateAuthorization({ ...approval(), approved: false }), /approve/);
  const first = validateAuthorization(approval());
  assert.deepEqual(validateAuthorization(first), first);
  assert.equal(first.budgetTokens, 100000);
  await assert.rejects(prepareScreen({ root: '/tmp/unused-v5-test-output', offlineRoot: '/tmp/no-offline' }), /authorization/);
});

test('authoring review must be independent, source-bound and complete', async () => {
  const root = await mkdtemp(join(tmpdir(), 'v5-portability-review-'));
  try {
    const files = {};
    for (const name of REVIEWED_FILES) { await writeFile(join(root, name), name); files[name] = sha256(name); }
    const review = { status: 'passed', independent: true, reviewer: 'Offline reviewer', providerCalls: 0, summary: 'Reviewed synthetic fixture contract', files };
    assert.deepEqual(await validateAuthoringReview(review, { sourceHere: root }), review);
    await assert.rejects(validateAuthoringReview({ ...review, independent: false }, { sourceHere: root }), /independent/);
    await assert.rejects(validateAuthoringReview({ ...review, files: {} }, { sourceHere: root }), /cover/);
    await writeFile(join(root, REVIEWED_FILES[0]), 'changed');
    await assert.rejects(validateAuthoringReview(review, { sourceHere: root }), /changed/);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('public entry points expose help without private files or provider dispatch', async () => {
  for (const name of ['validate-offline.mjs', 'run-screen.mjs']) {
    const { stdout, stderr } = await promisify(execFile)(process.execPath, [join(SOURCE_HERE, name), '--help'], { cwd: tmpdir(), timeout: 10000 });
    assert.match(stdout, /Usage:/);
    assert.equal(stderr, '');
  }
  const manifest = JSON.parse(await readFile(join(PACKAGE_ROOT, 'package.json'), 'utf8'));
  assert.match(manifest.scripts.test, /node --test/);
  assert.match(manifest.scripts.validate, /validate-offline/);
});

test('fresh frozen mock canary counts the new operator budget and refuses authorization mutation', { timeout: 30000 }, async () => {
  const { mkdir } = await import('node:fs/promises');
  const { snapshotImplementation } = await import('./package-support.mjs');
  const { buildCorpus, selectedScreenIds } = await import('./corpus.mjs');
  const { saveImmutable } = await import('./artifacts.mjs');
  const { verifyScreen } = await import('./freeze.mjs');
  const { executeScreen } = await import('./run-screen.mjs');
  const root = await mkdtemp(join(tmpdir(), 'v5-portable-freeze-'));
  try {
    const offline = join(root, 'offline'); await mkdir(offline);
    const corpus = buildCorpus(), files = {};
    for (const name of REVIEWED_FILES) files[name] = sha256(await readFile(join(SOURCE_HERE, name)));
    const review = { status: 'passed', independent: true, reviewer: 'Explicit offline mock only', providerCalls: 0,
      summary: 'Synthetic harness review fixture; not authorization for live benchmark use', files };
    const implementationFiles = await snapshotImplementation(offline);
    const validation = { passed: true, providerInvocations: 0, review, selectedCaseIds: selectedScreenIds, corpusHash: sha256(corpus), implementationFiles };
    await saveImmutable(offline, 'validation.json', validation);
    await saveImmutable(offline, 'validation.sha256.json', { sha256: sha256(validation) });
    await saveImmutable(offline, 'corpus.controller.json', corpus);
    const authorizationPath = join(root, 'approval.json'); await writeFile(authorizationPath, JSON.stringify(approval()));
    const runRoot = join(root, 'run');
    const prepared = await prepareScreen({ root: runRoot, offlineRoot: offline, authorizationPath, testOnly: true });
    assert.equal(prepared.manifest.budgetTokens, 100000);
    assert.equal((await verifyScreen(runRoot)).authorizationHash, sha256(validateAuthorization(approval())));
    await assert.rejects(prepareScreen({ root: runRoot, offlineRoot: offline, authorizationPath, testOnly: true }), /EEXIST/);
    let calls = 0;
    const result = await executeScreen({ root: runRoot, execute: true, skipCaffeinate: true, minStartIntervalMs: 0, canariesOnly: true,
      invoke: async ({ prompt }) => {
        calls++;
        const nonce = /The calibration token is ([A-Za-z0-9_-]+)\./.exec(prompt)?.[1];
        return { status: 'completed', answer: JSON.stringify(nonce ? { ack: nonce } : { token: null }),
          usage: { input_tokens: 8, output_tokens: 2, total_tokens: 10 }, events: [], violations: [] };
      } });
    assert.equal(calls, 2); assert.equal(result.tokens, 20); assert.equal(result.stopReason, null);
    const report = JSON.parse(await readFile(join(runRoot, result.reportPath), 'utf8'));
    assert.equal(report.budgetTokens, 100000);
    await writeFile(join(runRoot, 'authorization.json'), JSON.stringify({ ...validateAuthorization(approval()), budgetTokens: 200000 }));
    await assert.rejects(verifyScreen(runRoot), /authorization changed/);
  } finally { await rm(root, { recursive: true, force: true }); }
});


test('offline validation requires a nonzero actual test count in either Node output format', async () => {
  const { parseTestCount } = await import('./validate-offline.mjs');
  assert.equal(parseTestCount('ℹ tests 153\nℹ pass 153'), 153);
  assert.equal(parseTestCount('# tests 12\n# pass 12'), 12);
  assert.throws(() => parseTestCount('# tests 0'), /nonzero/);
  assert.throws(() => parseTestCount('no test summary'), /nonzero/);
});

test('comparison freeze rejects missing or mismatched real-native offline proof before creating a run', async()=>{
  const { mkdir }=await import('node:fs/promises');
  const { saveImmutable, exists }=await import('./artifacts.mjs');
  const root=await mkdtemp(join(tmpdir(),'v5-native-proof-'));
  try {
    const binary=join(root,'mock-native');await writeFile(binary,'offline fake binary');
    const config={executable:binary,sha256:sha256('offline fake binary'),version:'hivemind 0.7.0',backend:'hivemind-rust-cli',adapterVersion:'offline-test'};
    const auth=join(root,'authorization.json');await writeFile(auth,JSON.stringify({...approval(),profile:'comparison'}));
    for(const [name,nativeValidation] of [['absent',{status:'not-run'}],['wrong',{status:'passed',sha256:'f'.repeat(64),version:config.version,skippedTests:0}],['skipped',{status:'passed',...config,skippedTests:1}]]) {
      const offline=join(root,name);await mkdir(offline);
      const validation={passed:true,providerInvocations:0,nativeValidation};
      await saveImmutable(offline,'validation.json',validation);await saveImmutable(offline,'validation.sha256.json',{sha256:sha256(validation)});
      const run=join(root,`run-${name}`);
      await assert.rejects(prepareScreen({root:run,offlineRoot:offline,authorizationPath:auth,profile:'comparison',nativeBinary:binary,testOnly:true,nativeInspector:async()=>config}),/real-native offline tests/);
      assert.equal(await exists(run),false);
    }
    await assert.rejects(prepareScreen({root:join(root,'live-invalid'),offlineRoot:root,authorizationPath:auth,profile:'comparison',nativeBinary:binary,nativeInspector:async()=>config}),/non-overlapping|offline-test-only/);
  } finally {await rm(root,{recursive:true,force:true});}
});
