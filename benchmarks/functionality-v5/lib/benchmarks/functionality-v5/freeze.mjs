import { join, resolve } from 'node:path';
import { isDeepStrictEqual as same } from 'node:util';
import { VERSIONS, LIMITS, check } from './common.mjs';
import { PIN } from '../functionality-v3/subscription.mjs';
import { tokenizerManifest, countTokens } from '../functionality-v3/tokenizer.mjs';
import { saveImmutable, parse, verifyFiles, sha256 } from './artifacts.mjs';
import { PACKAGE_VERSION, createFreshDirectory, snapshotImplementation, validateOutputPath, validateAuthorization, validateAuthoringReview } from './package-support.mjs';

export const CONTROLLER_VERSION = 'functionality-v5-portable-screen-controller-1.0.0';

export async function prepareScreen({ root, offlineRoot, authorizationPath, testOnly = false } = {}) {
  check(typeof offlineRoot === 'string' && offlineRoot, 'Reviewed offline validation directory required');
  check(typeof authorizationPath === 'string' && authorizationPath, 'New explicit operator authorization file required');
  offlineRoot = resolve(offlineRoot);
  root = validateOutputPath(root, { offlineRoot });
  const authorization = validateAuthorization(await parse(resolve(authorizationPath)));
  const validation = await parse(join(offlineRoot, 'validation.json'));
  check(sha256(validation) === (await parse(join(offlineRoot, 'validation.sha256.json'))).sha256, 'Offline validation changed');
  check(validation.passed === true && validation.providerInvocations === 0, 'Provider-free offline validation must pass');
  await validateAuthoringReview(validation.review);
  const corpus = await parse(join(offlineRoot, 'corpus.controller.json'));
  const selected = validation.selectedCaseIds.map(id => corpus.find(scenario => scenario.id === id));
  check(selected.length === 3 && selected.every(Boolean), 'Exactly three screen cases required');
  check(sha256(corpus) === validation.corpusHash, 'Offline corpus hash changed');
  await verifyFiles(offlineRoot, validation.implementationFiles);
  await createFreshDirectory(root);
  await saveImmutable(root, 'corpus.controller.json', selected);
  await saveImmutable(root, 'offline-validation.json', validation);
  await saveImmutable(root, 'authorization.json', authorization);
  const files = await snapshotImplementation(root);
  for (const [name, digest] of Object.entries(validation.implementationFiles)) check(files[name] === digest, `Implementation changed after offline validation: ${name}`);
  const manifest = { controller: CONTROLLER_VERSION, packageVersion: PACKAGE_VERSION, versions: VERSIONS, model: PIN, limits: LIMITS,
    node: process.version, tokenizer: tokenizerManifest(), testOnly, budgetTokens: authorization.budgetTokens,
    createdAt: new Date().toISOString(), corpusHash: sha256(selected), files, implementationHash: sha256(files),
    cases: selected.map(scenario => ({ id: scenario.id, family: scenario.family, level: scenario.level,
      sourceSizingProxyTokens: countTokens(JSON.stringify(scenario.episodes)), sourceBytes: Buffer.byteLength(JSON.stringify(scenario.episodes)) })),
    offlineValidationHash: sha256(validation), authorizationHash: sha256(authorization),
    usageLabel: 'CLI-reported token usage', sizingNote: 'o200k_base is a pinned exposure sizing proxy, not verified model tokenization or provider usage.' };
  manifest.evaluationConditionsHash = sha256({ versions: VERSIONS, model: PIN, limits: LIMITS, corpusHash: manifest.corpusHash,
    implementationHash: manifest.implementationHash, measurementScope: 'v5-matched-posthandoff', arm: 'stock-maintained-notes' });
  await saveImmutable(root, 'manifest.json', manifest);
  await saveImmutable(root, 'manifest.sha256.json', { sha256: sha256(manifest) });
  return { root, manifest };
}

export async function verifyScreen(root) {
  const manifest = await parse(join(root, 'manifest.json'));
  check(manifest.controller === CONTROLLER_VERSION, 'Only fresh runs from this public package are supported');
  check(sha256(manifest) === (await parse(join(root, 'manifest.sha256.json'))).sha256, 'Frozen manifest changed');
  const authorization = validateAuthorization(await parse(join(root, 'authorization.json')));
  check(sha256(authorization) === manifest.authorizationHash && authorization.budgetTokens === manifest.budgetTokens, 'Frozen operator authorization changed');
  check(manifest.node === process.version && same(manifest.model, PIN), 'Frozen runtime settings changed');
  check(sha256(await parse(join(root, 'corpus.controller.json'))) === manifest.corpusHash, 'Frozen corpus changed');
  const validation = await parse(join(root, 'offline-validation.json'));
  check(sha256(validation) === manifest.offlineValidationHash, 'Offline validation changed');
  check(validation.passed && validation.review?.status === 'passed', 'Reviewed offline validation required');
  await verifyFiles(root, manifest.files);
  check(same(manifest.tokenizer, tokenizerManifest()), 'Pinned tokenizer changed');
  return manifest;
}
