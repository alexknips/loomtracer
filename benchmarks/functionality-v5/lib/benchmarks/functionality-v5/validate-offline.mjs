#!/usr/bin/env node
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { readdir } from 'node:fs/promises';
import { realpathSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { buildCorpus, selectedScreenIds } from './corpus.mjs';
import * as world from './world.mjs';
import { validateCorpus } from './validation.mjs';
import { saveImmutable, parse, sha256 } from './artifacts.mjs';
import { VERSIONS, LIMITS, check } from './common.mjs';
import { PACKAGE_ROOT, SOURCE_HERE, PACKAGE_VERSION, createFreshDirectory, validateOutputPath,
  validateAuthoringReview, snapshotImplementation, saveReviewTemplate } from './package-support.mjs';

export function parseTestCount(stdout) {
  const count = Number(/(?:ℹ|#) tests (\d+)/.exec(stdout)?.[1] ?? 0);
  check(Number.isSafeInteger(count) && count > 0, 'Offline validation must report a nonzero executed test count');
  return count;
}

export async function validateOffline({ output, reviewPath } = {}) {
  const root = validateOutputPath(output ?? join(PACKAGE_ROOT, '.artifacts', `offline-${new Date().toISOString().replaceAll(':', '-')}-${process.pid}`));
  const review = reviewPath ? await validateAuthoringReview(await parse(resolve(reviewPath))) : { status: 'pending', reason: 'No independent authoring review supplied' };
  const tests = (await readdir(SOURCE_HERE)).filter(name => name.endsWith('.test.mjs')).sort().map(name => join(SOURCE_HERE, name));
  const { stdout, stderr } = await promisify(execFile)(process.execPath, ['--test', ...tests], { cwd: PACKAGE_ROOT, maxBuffer: 8 * 1024 * 1024, timeout: 120000 });
  const corpus = buildCorpus(), checks = validateCorpus(corpus, { worldAdapter: world });
  check(checks.passed, JSON.stringify(checks.errors));
  await createFreshDirectory(root);
  await saveImmutable(root, 'tests.stdout.txt', stdout);
  await saveImmutable(root, 'tests.stderr.txt', stderr);
  await saveImmutable(root, 'authoring-review.json', review);
  await saveReviewTemplate(root);
  await saveImmutable(root, 'corpus.controller.json', corpus);
  await saveImmutable(root, 'corpus-validation.json', checks);
  const implementationFiles = await snapshotImplementation(root);
  const count = parseTestCount(stdout);
  const result = { version: 'functionality-v5-portable-offline-validation-1.0.0', packageVersion: PACKAGE_VERSION,
    passed: true, createdAt: new Date().toISOString(), versions: VERSIONS, limits: LIMITS,
    cases: corpus.length, selectedCaseIds: selectedScreenIds, corpusHash: sha256(corpus), testCount: count,
    testsPassed: true, corpusChecks: checks.checkCount, review, implementationFiles, implementationHash: sha256(implementationFiles),
    providerInvocations: 0, liveReadiness: review.status === 'passed' ? 'operator-authorization-and-fresh-canaries-required' : 'independent-authoring-review-required',
    limitation: 'Offline reference trajectories establish feasibility and contract consistency, not participant accuracy, semantic validity, or calibrated difficulty. No historical private runs are required or modified.' };
  await saveImmutable(root, 'validation.json', result);
  await saveImmutable(root, 'validation.sha256.json', { sha256: sha256(result) });
  return { root, passed: true, testCount: count, cases: corpus.length, corpusChecks: checks.checkCount,
    providerInvocations: 0, authoringReview: review.status, liveReadiness: result.liveReadiness };
}

export const HELP = `Provider-free v5 validation\n\nUsage: npm run validate -- [--output /absolute/new/directory] [--review /path/review.json]\n\nRuns all local tests and 27-case corpus/world checks, then freezes immutable sources.\nOmit --output to create a fresh timestamped .artifacts/offline-* directory. Existing\noutput directories are refused. No model calls, authentication, or private run data\nare needed. Install the pinned tokenizer first with npm ci from this package.\n\nAn independent semantic authoring review is a separate requirement before live use.\nThe output includes authoring-review.template.json with current source hashes. A\nreviewer must inspect those sources and record independent=true, reviewer, summary,\nstatus=passed and providerCalls=0. Validate again into a NEW directory with --review\nto bind that review to the exact implementation. A successful local test run does\nnot automatically approve semantic correctness or authorize provider expenditure.\n`;
const main = () => { try { return realpathSync(process.argv[1]) === fileURLToPath(import.meta.url); } catch { return false; } };
if (main()) {
  const { values } = parseArgs({ options: { output: { type: 'string' }, review: { type: 'string' }, help: { type: 'boolean', short: 'h' } } });
  if (values.help) console.log(HELP);
  else console.log(JSON.stringify(await validateOffline({ output: values.output, reviewPath: values.review })));
}
