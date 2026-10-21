import { buildCorpus, bindAcquisition, eligibleSources, selectedScreenIds } from './corpus.mjs';
import { DIMENSIONS } from './scoring.mjs';
import { LIMITS } from './common.mjs';

export const VALIDATION_VERSION = 'functionality-v5-offline-validation-1.0.1';
const unique = xs => new Set(xs).size === xs.length;
const expectedUnits = { easy: 12, intermediate: 16, demanding: 20 };
const textBytes = value => Buffer.byteLength(JSON.stringify(value), 'utf8');
const contractPublic = scenario => JSON.stringify({ task: scenario.task, contract: scenario.contract, questions: scenario.questions, acquisition: scenario.acquisition, episode1: scenario.episodes[0] });

/** Narrow authored-fixture check, not a general natural-language entailment grader.
 * The assertion is defined here independently of the evidence pointer. A quoted
 * substring or a value in the private answer key cannot establish this role.
 * A source speaker explicitly declares the role in the originating production
 * occurrence, before maintenance; later/noise messages cannot repair it silently.
 */
export function validateProducerEvidence(scenario) {
  if (scenario.family !== 'release') return { applicable: false, passed: true, errors: [] };
  const evidence = scenario.worldDefinition?.knownFieldEvidence?.producer;
  const expected = scenario.worldDefinition?.knownFields?.producer;
  const episode = scenario.episodes?.find(e => e.number === 1);
  const occurrence = episode?.sources?.find(s => s.id === evidence?.sourceId);
  const errors = [];
  if (evidence?.sourceId !== 'src-0001' || evidence?.basis !== 'speaker-role-declaration') errors.push('Explicit originating producer binding required');
  if (!occurrence || occurrence.conversation !== 'production') errors.push('Producer binding must target the originating production occurrence');
  if (typeof expected !== 'string' || !expected.trim() || occurrence?.actor !== expected) errors.push('Producer value must equal the actor declaring the role');
  const sentences = String(occurrence?.text ?? '').split(/(?<=[.!?])\s+/).map(s => s.trim());
  const roleStatements = sentences.filter(s => /\bproducer\b|\blocale validation\b/i.test(s));
  if (roleStatements.length !== 1 || roleStatements[0] !== 'I am the producer for Aurora locale validation.') {
    errors.push('One unqualified first-person Aurora locale-validation producer declaration required');
  }
  return { applicable: true, passed: errors.length === 0, errors, sourceId: evidence?.sourceId ?? null };
}

/** Corpus and world validation is deterministic. No invocation adapter is accepted. */
export function validateCorpus(corpus = buildCorpus(), { worldAdapter = null } = {}) {
  const checks = [], errors = [], caseChecks = [];
  const verify = (condition, name, details = null) => { const row = { name, passed: !!condition, ...(details === null ? {} : { details }) }; checks.push(row); if (!condition) errors.push(name); return condition; };
  verify(Array.isArray(corpus) && corpus.length === 27, '27 authored cases');
  verify(unique(corpus.map(s => s.id)), 'Unique case identifiers');
  verify(selectedScreenIds.every(id => corpus.some(s => s.id === id)), 'All approved screen cases exist');
  verify(corpus.every(s => ['incident', 'experiment', 'release'].includes(s.family) && ['easy', 'intermediate', 'demanding'].includes(s.level) && ['same', 'distinct', 'unresolved'].includes(s.identity)), 'Valid family difficulty identity cells');
  for (const family of ['incident', 'experiment', 'release']) for (const level of ['easy', 'intermediate', 'demanding']) {
    const siblings = corpus.filter(s => s.family === family && s.level === level);
    verify(siblings.length === 3 && unique(siblings.map(s => s.identity)), `${family}/${level}: complete counterfactual identity siblings`);
    verify(new Set(siblings.map(contractPublic)).size === 1, `${family}/${level}: current public request and contract identical across identity siblings`);
  }
  verify(new Set(['incident', 'experiment', 'release'].map(f => {
    const s = corpus.find(s => s.family === f && s.level === 'demanding');
    return JSON.stringify(s?.worldDefinition?.fresh?.map(x => [x.prerequisites.length, x.fields.length, x.identityDependent]));
  })).size >= 2, 'Families have distinct dependency structures');

  for (const scenario of corpus) {
    const start = checks.length, prefix = `${scenario.id}: `;
    const sources = scenario.episodes.flatMap(e => e.sources);
    const futureAcquisitionSources = new Set([
      ...(scenario.worldDefinition?.acquisitionChecks ?? []).map(c => `acq-${c.operation}`),
      `acq-${scenario.worldDefinition?.original?.operation}`, 'origin-summary',
    ]);
    const present = new Set(sources.map(s => s.id));
    verify(scenario.episodes.length === 4 && scenario.episodes.every((e, i) => e.number === i + 1), prefix + 'Four chronological episodes');
    verify(unique(sources.map(s => s.id)) && sources.every(s => s.id && s.actor && s.conversation && typeof s.text === 'string' && s.text.trim()), prefix + 'Unique source occurrences with actor conversation and text');
    const producerEvidence = validateProducerEvidence(scenario);
    if (producerEvidence.applicable) verify(producerEvidence.passed, prefix + 'Producer requirement is grounded in an explicit eligible role declaration', producerEvidence.errors);
    verify(scenario.hUnits.length === expectedUnits[scenario.level] && DIMENSIONS.every(d => scenario.hUnits.filter(u => u.dimension === d).length === expectedUnits[scenario.level] / 4), prefix + 'Balanced strict H dimensions and difficulty count');
    verify(unique(scenario.hUnits.map(u => u.id)) && unique(scenario.qUnits.map(u => u.id)) && unique(scenario.wUnits.map(u => u.id)), prefix + 'Unique obligations');
    verify(scenario.hUnits.every(u => scenario.questions.some(q => q.id === u.questionId)), prefix + 'Every H unit maps to a public question');
    verify(scenario.hUnits.every(u => u.sourceIds.length && u.sourceIds.every(id => present.has(id) || futureAcquisitionSources.has(id))), prefix + 'Every H unit has eligible source binding or declared acquisition occurrence');
    verify(scenario.hUnits.every(u => (u.dependsOn ?? []).every(id => id !== u.id && scenario.hUnits.some(other => other.id === id))), prefix + 'H semantic dependencies reference frozen units');
    verify(scenario.qUnits.filter(u => u.guard).length === 1 && scenario.qUnits.find(u => u.guard)?.kind === 'avoid', prefix + 'One fixed-weight extraneous question guard');
    verify(scenario.wUnits.filter(u => u.guard).length === 1 && scenario.wUnits.find(u => u.guard)?.kind === 'reuse', prefix + 'One fixed-weight extraneous work guard');
    verify(scenario.qUnits.every(u => ['necessary', 'avoid'].includes(u.kind)) && scenario.wUnits.every(u => ['reuse', 'fresh'].includes(u.kind)), prefix + 'Explicit applicable behavior strata');
    verify(scenario.identityEvidence.classification === scenario.identity && scenario.identityEvidence.sourceIds.every(id => present.has(id)), prefix + 'Identity truth source-backed');
    verify(!/v5-(incident|experiment|release)-(easy|intermediate|demanding)-(same|distinct|unresolved)/.test(JSON.stringify({ task: scenario.task, contract: scenario.contract, questions: scenario.questions })), prefix + 'Public contract hides design identifiers');
    verify(scenario.episodes.filter(e => [2, 4].includes(e.number)).every(e => e.sources.length >= 8), prefix + 'Both intervening episodes include distractors');
    verify(textBytes(scenario.episodes[0].sources) <= LIMITS.episodeBytes && scenario.episodes.every(e => textBytes(e.sources) <= LIMITS.episodeBytes), prefix + 'New episode deliveries fit declared limit');
    verify(scenario.controlExpectation?.missing && scenario.controlExpectation?.expected && scenario.withheldSourceIds?.length, prefix + 'Frozen narrow withheld expectation');
    const withheld = eligibleSources(scenario, { withheld: true });
    verify(withheld.every(s => !scenario.withheldSourceIds.includes(s.id)), prefix + 'Withheld oracle view removes declared decisive sources');
    // Every counterfactual class assertion is confined to the registry occurrence
    // before acquisition. Generic public applicability rules are not answers.
    if (scenario.withheldSourceIds.includes('src-0011')) {
      const registry = sources.find(s => s.id === 'src-0011');
      verify(registry && !withheld.some(s => s.text === registry.text), prefix + 'Identity registry occurrence absent from withheld source view');
    }
    const row = { id: scenario.id, H: scenario.hUnits.length, Q: scenario.qUnits.length, W: scenario.wUnits.length,
      questionStrata: Object.fromEntries(['necessary', 'avoid'].map(k => [k, scenario.qUnits.filter(u => u.kind === k).length])),
      workStrata: Object.fromEntries(['reuse', 'fresh'].map(k => [k, scenario.wUnits.filter(u => u.kind === k).length])),
      sourceBytes: textBytes(sources), checks: checks.length - start };
    caseChecks.push(row);
  }

  if (!worldAdapter) verify(false, 'World transition and valid-plan adapter required for final validation');
  else {
    const required = ['createWorld', 'applyAction', 'validAcquisitionPlan', 'validActionPlan', 'alternateActionPlan', 'deterministicChecks'];
    if (verify(required.every(k => typeof worldAdapter[k] === 'function'), 'Complete world validation adapter')) {
      for (const scenario of corpus) {
        const row = caseChecks.find(c => c.id === scenario.id);
        try {
          const acquisition = worldAdapter.createWorld(scenario, { phase: 'acquisition' });
          const acquisitionPlan = worldAdapter.validAcquisitionPlan(scenario);
          for (const action of acquisitionPlan) worldAdapter.applyAction(acquisition, action);
          const acqChecks = worldAdapter.deterministicChecks(scenario, acquisition);
          verify(acqChecks.complete === true, `${scenario.id}: originating reference plan completes`);
          const bound = bindAcquisition(scenario, acquisition, { summary: 'Offline fixture originating handoff: no additional rationale asserted.' });
          verify(bound.sources.every((s, i) => s.order === i + 1) && unique(bound.sources.map(s => s.id)), `${scenario.id}: immutable source occurrence ordering after acquisition`);
          verify(bound.scenario.hUnits.every(u => u.sourceAvailability.every(s => s.available)), `${scenario.id}: successful acquisition binds every original H occurrence`);
          const plan = worldAdapter.validActionPlan(bound.scenario);
          const operations = plan.length;
          row.referencePlanOperations = operations;
          verify(operations <= LIMITS.brokerSteps - LIMITS.planReserve, `${scenario.id}: reference plan leaves four operations for lookup/exploration`, { operations, reserve: LIMITS.brokerSteps - operations });
          const access = sourceRows => ({ eligibleSourceIds: sourceRows.map(s => s.id),
            visibleArtifactIds: Object.values(acquisition.artifacts).filter(a => sourceRows.some(s => s.id === `acq-${a.operation}`)).map(a => a.id) });
          const world = worldAdapter.createWorld(bound.scenario, { phase: 'action', acquisition, ...access(bound.sources) });
          verify(plan.some(action => action.op === 'lookup'), `${scenario.id}: plan explicitly counts minimum historical lookup`);
          for (const action of plan.filter(action => action.op !== 'lookup')) worldAdapter.applyAction(world, action);
          const result = worldAdapter.deterministicChecks(bound.scenario, world);
          verify(result.complete === true, `${scenario.id}: reference successor plan reaches correct terminal state`, result.errors);
          verify([...(result.Q ?? []), ...(result.W ?? [])].every(u => u.status === 'pass'), `${scenario.id}: valid reference plan passes every applicable local milestone`);
          row.terminal = result.terminal;
          if (typeof worldAdapter.alternateActionPlan === 'function') {
            const alternative = worldAdapter.createWorld(bound.scenario, { phase: 'action', acquisition, ...access(bound.sources) });
            for (const action of worldAdapter.alternateActionPlan(bound.scenario).filter(action => action.op !== 'lookup')) worldAdapter.applyAction(alternative, action);
            verify(worldAdapter.deterministicChecks(bound.scenario, alternative).complete === true, `${scenario.id}: independent valid operation order also completes`);
          } else verify(false, `${scenario.id}: alternative valid order fixture required`);
          const reduced = eligibleSources(bound.scenario, { withheld: true });
          verify(reduced.every(s => !bound.scenario.withheldSourceIds.includes(s.id)), `${scenario.id}: bound withheld view excludes decisive acquisition traces`);
          const withheldWorld = worldAdapter.createWorld(bound.scenario, { phase: 'action', acquisition, withheld: true, ...access(reduced) });
          for (const action of worldAdapter.validActionPlan(bound.scenario, { withheld: true }).filter(action => action.op !== 'lookup')) worldAdapter.applyAction(withheldWorld, action);
          const withheldChecks = worldAdapter.deterministicChecks(bound.scenario, withheldWorld);
          verify(withheldChecks.complete === true && withheldChecks.correctlyBlocked === true,
            `${scenario.id}: regenerated withheld plan completes safe work and blocks dependent completion`, withheldChecks.errors);
          row.oracleBytes = textBytes(bound.sources); row.withheldBytes = textBytes(reduced);
          verify(row.oracleBytes <= LIMITS.historicalPerTask, `${scenario.id}: compact oracle evidence can fit historical allowance`);
        } catch (error) { verify(false, `${scenario.id}: world validation exception`, error.message); }
      }
    }
  }
  return { version: VALIDATION_VERSION, passed: errors.length === 0, checkCount: checks.length, checks, errors, caseChecks,
    limitations: ['Offline reference trajectories establish feasibility, not expected live performance.', 'Semantic review remains required; deterministic tests do not infer meaning from keywords.', 'No provider, canary or model invocation occurs in this validator.'] };
}

export const offlineValidation = validateCorpus;
