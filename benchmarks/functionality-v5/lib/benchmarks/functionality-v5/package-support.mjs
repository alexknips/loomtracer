import { mkdir, readFile } from 'node:fs/promises';
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { APPROVED_SCREEN_BUDGET, check } from './common.mjs';
import { saveImmutable, snapshotTree, sha256 } from './artifacts.mjs';

export const SOURCE_HERE = dirname(fileURLToPath(import.meta.url));
export const PACKAGE_ROOT = resolve(SOURCE_HERE, '../../..');
export const SUPPORT_TREES = ['functionality-v5', 'functionality-v3', 'functionality'];
export const REVIEWED_FILES = ['common.mjs', 'corpus.mjs', 'world.mjs', 'validation.mjs', 'scoring.mjs', 'grading.mjs', 'workflow.mjs', 'memory.mjs'];
export const PACKAGE_VERSION = 'functionality-v5-portable-package-1.0.0';
const inside = (root, child) => { const value = relative(root, child); return value === '' || (!isAbsolute(value) && value !== '..' && !value.startsWith(`..${sep}`)); };

export function validateOutputPath(output, { sourceHere = SOURCE_HERE, offlineRoot } = {}) {
  check(typeof output === 'string' && output.length > 0, 'Fresh output path required');
  const root = resolve(output), sources = resolve(sourceHere, '..');
  check(!inside(sources, root), 'Output must be outside the implementation source tree');
  if (offlineRoot) check(!inside(resolve(offlineRoot), root) && !inside(root, resolve(offlineRoot)), 'Run and offline validation need separate non-overlapping directories');
  return root;
}

export function validateAuthorization(value) {
  check(value && typeof value === 'object' && !Array.isArray(value), 'Explicit operator authorization JSON required');
  check(value.approved === true, 'Operator must explicitly approve this fresh run');
  check(typeof value.operator === 'string' && value.operator.trim().length > 0, 'Named operator required');
  check(typeof value.purpose === 'string' && value.purpose.trim().length > 0, 'Run purpose required');
  check(typeof value.approvedAt === 'string' && Number.isFinite(Date.parse(value.approvedAt)), 'Authorization timestamp required');
  check(Number.isSafeInteger(value.budgetTokens) && value.budgetTokens > 0 && value.budgetTokens <= APPROVED_SCREEN_BUDGET,
    'Operator token budget must be positive and no greater than 4,250,000');
  return { operator: value.operator.trim(), approved: true, purpose: value.purpose.trim(), approvedAt: value.approvedAt,
    budgetTokens: value.budgetTokens, appliesTo: 'one-fresh-maintained-notes-screen',
    scope: ['primary', 'oracle', 'withheld', 'grading', 'canary', 'shared-acquisition'],
    usageLabel: 'CLI-reported token usage',
    threshold: 'Stops subsequent dispatch; not a hard provider cap. Any failed or unknown-usage call stops dispatch.' };
}

export async function validateAuthoringReview(value, { sourceHere = SOURCE_HERE } = {}) {
  check(value && value.status === 'passed' && value.providerCalls === 0, 'Independent offline authoring review must pass without provider calls');
  check(value.independent === true && typeof value.reviewer === 'string' && value.reviewer.trim(), 'Named independent reviewer required');
  check(typeof value.summary === 'string' && value.summary.trim(), 'Authoring review findings required');
  check(value.files && typeof value.files === 'object', 'Source-bound review hashes required');
  for (const name of REVIEWED_FILES) check(typeof value.files[name] === 'string', `Authoring review must cover ${name}`);
  for (const [name, digest] of Object.entries(value.files)) {
    check(!isAbsolute(name) && inside(sourceHere, resolve(sourceHere, name)), 'Review file path must stay inside v5 sources');
    check(/^[a-f0-9]{64}$/.test(digest), `Invalid review hash: ${name}`);
    check(sha256(await readFile(resolve(sourceHere, name))) === digest, `Reviewed source changed: ${name}`);
  }
  return structuredClone(value);
}

export async function snapshotImplementation(root, { sourceHere = SOURCE_HERE } = {}) {
  const files = {};
  for (const name of SUPPORT_TREES) Object.assign(files, await snapshotTree({ source: resolve(sourceHere, '..', name), root, target: `snapshot/benchmarks/${name}` }));
  return files;
}

export async function createFreshDirectory(root) {
  await mkdir(dirname(root), { recursive: true });
  await mkdir(root, { recursive: false, mode: 0o700 });
}

export async function saveReviewTemplate(root, { sourceHere = SOURCE_HERE } = {}) {
  const files = Object.fromEntries(await Promise.all(REVIEWED_FILES.map(async name => [name, sha256(await readFile(join(sourceHere, name)))])));
  const value = { status: 'pending', independent: false, reviewer: '', providerCalls: 0, summary: '', files };
  await saveImmutable(root, 'authoring-review.template.json', value);
  return value;
}
