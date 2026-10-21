import { createHash } from 'node:crypto';

const clone = value => structuredClone(value);
const hash = value => createHash('sha256').update(JSON.stringify(value)).digest('hex');
// A versioned tool-input contract also applies when projecting a frozen legacy
// scenario. It describes representation requirements, never historical values.
const SHIFT_INPUT_DESCRIPTION = 'Verify the supplied operator initials against the safe enclosure receipt and the retained shift scope. The shift input must state both staffing and time of day together, using values established in retained sources. Supply one string containing those two phrases in either order, separated by whitespace, a comma or a semicolon; optional shift and on-site words are allowed. A staffing count may use its numeral or word form. Missing parts, alternative values, negation and extra qualifiers are not accepted.';
/** A closed representation grammar, not keyword matching over arbitrary prose. */
function parseShift(value) {
  if (typeof value !== 'string' || value.length > 160) return null;
  let remaining = value.trim().toLowerCase().replace(/[‐‑‒–—−]/g, '-');
  const counts = { staffing: 0, timeOfDay: 0, shift: 0, onsite: 0 };
  while (remaining) {
    const token = /^(one[ -]+technician|1[ -]+technician|daytime|shift|on[- ]?site)(?=$|[\s,;])/.exec(remaining);
    if (!token) return null;
    const kind = /^(one|1)[ -]+technician$/.test(token[1]) ? 'staffing' : token[1] === 'daytime' ? 'timeOfDay' : token[1] === 'shift' ? 'shift' : 'onsite';
    if (++counts[kind] > 1) return null;
    remaining = remaining.slice(token[0].length);
    if (remaining) {
      const separator = /^[\s,;]+/.exec(remaining);
      if (!separator) return null;
      remaining = remaining.slice(separator[0].length);
    }
  }
  return counts.staffing === 1 && counts.timeOfDay === 1 ? { staffing: 1, timeOfDay: 'daytime' } : null;
}
const equivalentValue = (field, actual, expected) => {
  if (actual === undefined || actual === null || expected === undefined || expected === null) return false;
  if (field === 'shift') {
    const actualMeaning = parseShift(actual), expectedMeaning = parseShift(expected);
    return !!actualMeaning && !!expectedMeaning && actualMeaning.staffing === expectedMeaning.staffing && actualMeaning.timeOfDay === expectedMeaning.timeOfDay;
  }
  if (typeof expected === 'number') return Number(actual) === expected;
  const normalize = x => String(x).toLowerCase().replaceAll('two', '2').replaceAll('one', '1').replaceAll('–', '-').replaceAll('—', '-').replace(/[^a-z0-9]/g, '');
  if (field === 'staffing') return /^(2|2people|2person|2persons|2personstaffing)$/.test(normalize(actual));
  return normalize(actual) === normalize(expected);
};
const stateFor = world => world._scenario.worldDefinition;
const effectiveIdentity = world => world.withheld && world._scenario.withheldSourceIds.includes('src-0011') ? 'unresolved' : world._scenario.identity;
const neededFresh = world => stateFor(world).fresh.filter(x => effectiveIdentity(world) !== 'unresolved' || !x.identityDependent);
const operation = (world, name) => [...stateFor(world).acquisitionChecks, stateFor(world).original, stateFor(world).acquisitionFinal, ...stateFor(world).fresh].filter(Boolean).find(x => x.operation === name);

export function createWorld(scenario, { phase = 'action', acquisition = null, withheld = false, eligibleSourceIds = [], visibleArtifactIds = [] } = {}) {
  if (!['acquisition', 'action'].includes(phase)) throw new Error('Invalid world phase');
  const prior = acquisition?.artifacts || acquisition?.world?.artifacts || {};
  return {
    scenarioId: scenario.id, phase, withheld, status: 'active', step: 0,
    actions: [], receipts: [], artifacts: {}, priorArtifacts: clone(prior),
    available: {}, questionAnswers: {}, questionEvents: [], reused: [], repeated: [],
    identityDecision: null, outwardStatements: [], firstSubstantiveStep: null,
    terminal: null, errors: [], _scenario: clone(scenario),
    eligibleSourceIds: [...eligibleSourceIds], visibleArtifactIds: [...visibleArtifactIds],
  };
}

/** Only this object goes to evaluated agents. Never serialize the private world. */
export function publicWorldContract(scenario, phase = 'action') {
  const d = scenario.worldDefinition;
  const operations = phase === 'acquisition'
    ? [...d.acquisitionChecks, d.original, d.acquisitionFinal].filter(Boolean).map(x => ({ operation: x.operation, prerequisites: x.prerequisites || [], description: x.operation === d.original.operation ? 'Execute the original bounded comparison/check using its original scope and actual prerequisite receipts.' : `Execute ${x.operation.replaceAll('_', ' ')} and return a measured observation artifact.` }))
    : d.fresh.map(x => ({ operation: x.operation, prerequisites: x.prerequisites, fields: x.fields,
      description: x.publicDescription ?? (x.operation === 'verify_operator' ? SHIFT_INPUT_DESCRIPTION : x.description), identityDependent: x.identityDependent }));
  return {
    protocol: 'functionality-v5-world-1.0.2', phase,
    rules: scenario.contract.rules,
    operations,
    conditionalClarification: phase === 'action' ? d.conditional?.rule || null : null,
    actions: {
      run: '{op:"run",operation:string,artifacts?:string[],inputs?:object,message?:string}. Each prerequisite artifact must already have a successful receipt; fields must use actually supplied values.',
      ask: '{op:"ask",fields:string[],message?:string}. Batch independent fields only. A field whose branch depends on an earlier answer must wait for that answer. Historical-record and identity answers are not available through this respondent.',
      reuse: '{op:"reuse",artifactId:string,message?:string}. Refer to a prior artifact established in retained memory; the operation checks applicability and returns a current use receipt, not the historical record.',
      message: '{op:"message",text:string}. Optional outward statement. You may instead include optional message text with another operation.',
      recognize: '{op:"recognize",classification:"same"|"distinct"|"unresolved",evidence:string[],explanation:string}. Optional record of a scoped case assessment; it does not execute work or reveal whether the assessment is correct.',
      finish: '{op:"finish",summary:string,artifacts:string[]}. Cite actual artifact identifiers or receipt identifiers. All applicable operations must have succeeded. Correct references in the summary may also be represented as {artifactId:string} objects in artifacts.',
      block: '{op:"block",reason:string,artifacts:string[]}. Finish safe independent work first; identify the missing evidence/authorization. Blocking is not a resolved-work completion.',
    },
    limits: { brokerOperations: 16, sideEffects: 'Synthetic local world only' },
    notes: phase === 'action' ? 'Tools can also repeat an original check named by retained evidence, but each repetition counts as substantive work. The current request does not grant archive search or access to unretained historical artifacts.' : 'Produce your own concise final work handoff with your stated rationale. No successful handoff is prewritten.',
  };
}

function fail(world, receipt, code, detail) {
  receipt.ok = false;
  receipt.error = { code, detail };
  world.errors.push({ step: world.step, code, detail });
  return receipt;
}

function addArtifact(world, receipt, item, observation, inputs = {}, prerequisites = []) {
  const id = item.artifactId;
  const artifact = {
    id, kind: item.kind || item.operation, operation: item.operation,
    observation, applicability: item.applicability || 'Only the scoped inputs and prerequisites recorded in this receipt.',
    inputs: clone(inputs), prerequisites: [...prerequisites], receiptId: receipt.id,
    scope: world.phase === 'acquisition' ? 'original-work' : 'current-work',
  };
  artifact.hash = hash(artifact);
  world.artifacts[id] = artifact;
  world.available[item.baseArtifactId || id] = id;
  receipt.artifact = clone(artifact);
  receipt.operation = item.operation;
  return receipt;
}

function artifactReference(world, ref) {
  const key = typeof ref === 'string' ? ref : ref?.artifactId || ref?.receiptId;
  if (!key) return null;
  if (world.artifacts[key]) return key;
  const receipt = world.receipts.find(x => x.id === key && x.ok);
  return receipt?.artifact?.id || receipt?.usedArtifactId || null;
}

function actualCurrentObservation(world, spec, inputs) {
  const family = world._scenario.family;
  const op = spec.operation;
  const identity = effectiveIdentity(world);
  const scope = identity === 'same' ? 'original mapped target' : 'separately mapped current target';
  const observations = {
    configure_trial: `Current configuration: Cedar b17, schema C3, staging, eight jobs per batch, poolCap ${inputs.pool_cap}; production excluded.`,
    run_current_canary: `Executed eight-job staging canary for ${inputs.trial_minutes} minutes using the current-config receipt: 99 of 100 requests met target; no production action occurred.`,
    verify_alert: `Current alert route verified to ${inputs.alert_recipient}; restore-b17 rollback use is linked to its existing check.`,
    record_trial: 'Created current staging trial record with current-canary and alert-check receipts. Eight-job limit, two-person staffing and production exclusion retained.',
    prepare_batch: `Prepared six-sample batch for ${scope} on K4/2.6 with buffer ${inputs.buffer} and volume ${inputs.volume_ul} microliters.`,
    analyze_batch: 'Executed fresh six-sample analysis from prepared-batch receipt: five detected samples, one negative, all controls acceptable. This is a batch result, not a long-term clinical outcome.',
    verify_operator: `Verified operator ${inputs.operator} for the one-technician daytime controlled batch against the safety-check receipt.`,
    package_analysis: 'Created current analysis package from actual batch-analysis and operator-verification receipts. Controlled scope retained; no unrestricted population approval inferred.',
    validate_current_locale: `Executed zh-Hant string, line-break and glyph checks with font profile ${inputs.font_profile}: all 24 strings and all required glyphs pass. No distribution occurred.`,
    sign_validation: `Validator ${inputs.validator} signed the actual zh-Hant validation receipt for internal preparation. This is not deliverable-specific distribution authority.`,
    prepare_delivery_folder: `Prepared empty internal-preview folder alias ${inputs.folder_alias}; no assets distributed.`,
    assemble_release_manifest: 'Created a release manifest linking the actual current validation signoff and delivery-folder receipts to the established deliverable authorization and unchanged Tuesday window. Public distribution excluded.',
  };
  if (!observations[op]) throw new Error(`No current observation for ${family}/${op}`);
  return observations[op];
}

/** Each call counts one operation, including illegal or rejected operations. */
export function applyAction(world, action, { batchStart = null, eligibleSourceIds = world.eligibleSourceIds, visibleArtifactIds = world.visibleArtifactIds } = {}) {
  const receipt = { id: `receipt-${String(world.step + 1).padStart(3, '0')}`, step: ++world.step, ok: true, operation: action?.op || null };
  const beforeAnswers = clone(world.questionAnswers);
  const d = stateFor(world);
  const sourceEligibility = new Set(eligibleSourceIds || []);
  const artifactVisibility = new Set(visibleArtifactIds || []);
  const record = () => {
    world.receipts.push(clone(receipt));
    world.actions.push({ step: world.step, action: clone(action), receipt: clone(receipt) });
    return clone(receipt);
  };
  if (!action || typeof action !== 'object') { fail(world, receipt, 'invalid_action', 'Expected an action object'); return record(); }
  if (world.terminal) { fail(world, receipt, 'already_terminal', 'This fork already ended'); return record(); }
  if (world.step > 16) { fail(world, receipt, 'operation_limit', 'The 16-operation allowance is exhausted'); world.status = 'failed'; return record(); }
  if (batchStart) {
    const spec = action.op === 'run' || action.op === 'observe' ? operation(world, action.operation) : null;
    const beforeAvailable = batchStart.available || {};
    const beforeQuestions = batchStart.questionAnswers || {};
    const c = d.conditional;
    const dependent = spec?.prerequisites?.some(id => !beforeAvailable[id] && world.available[id]) || spec?.fields?.some(id => beforeQuestions[id] === undefined && world.questionAnswers[id] !== undefined) || (action.op === 'ask' && c && action.fields?.includes(c.field) && beforeQuestions[c.dependsOnField] === undefined && world.questionAnswers[c.dependsOnField] !== undefined);
    const dependentFinish = ['finish', 'block'].includes(action.op) && world.receipts.some(r => r.ok && (r.artifact || r.usedArtifactId) && r.step > (batchStart.step || 0));
    if (dependent || dependentFinish) { fail(world, receipt, 'dependent_batch', 'Dependent operations and final output must wait for the prerequisite receipt in a later invocation'); return record(); }
  }
  if (typeof action.message === 'string') world.outwardStatements.push({ step: world.step, text: action.message, beforeOperation: true });
  const op = action.op;
  if (op === 'message') {
    if (typeof action.text !== 'string' || !action.text.trim()) fail(world, receipt, 'empty_message', 'Message text is required');
    else { world.outwardStatements.push({ step: world.step, text: action.text, beforeOperation: true }); receipt.messageRecorded = true; }
  } else if (op === 'recognize') {
    if (!['same', 'distinct', 'unresolved'].includes(action.classification) || !Array.isArray(action.evidence) || !action.explanation?.trim()) fail(world, receipt, 'invalid_assessment', 'Supply classification, source evidence identifiers and an explanation');
    else {
      world.identityDecision = { classification: action.classification, evidence: clone(action.evidence), explanation: action.explanation, atStep: world.step };
      receipt.assessmentRecorded = true;
    }
  } else if (op === 'ask') {
    if (!Array.isArray(action.fields) || !action.fields.length) fail(world, receipt, 'invalid_question', 'Supply one or more field names');
    else {
      receipt.answers = {};
      for (const field of action.fields) {
        let status = 'answered';
        let answer;
        const conditional = d.conditional?.field === field ? d.conditional : null;
        const relevant = neededFresh(world).some(x => x.fields.includes(field)) || (d.conditional?.dependsOnField === field && neededFresh(world).some(x => x.fields.includes(d.conditional.field)));
        if (['identity', 'mapping', 'case_id', 'study_id', 'deliverable_id'].includes(field)) { status = 'unavailable'; answer = 'The respondent has no additional historical discriminator. Use eligible retained evidence; absent mapping remains unresolved.'; }
        else if (conditional && beforeAnswers[conditional.dependsOnField] === undefined) { status = 'premature'; answer = `This field depends on an earlier ${conditional.dependsOnField} answer; it cannot be requested speculatively in the same question.`; }
        else if (conditional && beforeAnswers[conditional.dependsOnField] !== conditional.equals) { status = 'inactive'; answer = 'This branch is inactive for the supplied discriminator.'; }
        else if (Object.hasOwn(world.questionAnswers, field)) { status = 'repeated'; answer = 'That field was already answered in this fork.'; }
        else if (Object.hasOwn(d.knownFields, field)) { status = 'already_known'; answer = 'That applicable field is already established in the eligible history; the respondent does not replay historical messages.'; }
        else if (d.inactiveFields.includes(field) || (Object.hasOwn(d.unknownFields, field) && !relevant)) { status = 'inactive'; answer = 'That field belongs to an inactive or blocked branch.'; }
        else if (!Object.hasOwn(d.unknownFields, field)) { status = 'extraneous'; answer = 'No scripted respondent answer exists for that field.'; }
        else { answer = d.unknownFields[field]; world.questionAnswers[field] = answer; }
        receipt.answers[field] = { status, value: answer };
        world.questionEvents.push({ field, status, step: world.step, receiptId: receipt.id });
      }
    }
  } else if (op === 'reuse') {
    if (world.firstSubstantiveStep === null) world.firstSubstantiveStep = world.step;
    const spec = d.reuse.find(x => x.artifactId === action.artifactId);
    const prior = world.priorArtifacts[action.artifactId];
    if (world.phase !== 'action') fail(world, receipt, 'no_prior_work', 'Originating work has no prior artifact to reuse');
    else if (!spec) fail(world, receipt, 'stale_or_inapplicable', 'The referenced artifact does not satisfy a portable current prerequisite');
    else if (world.withheld && world._scenario.controlExpectation.blockedReuse?.includes(spec.artifactId)) fail(world, receipt, 'historical_evidence_unavailable', 'The eligible evidence does not establish this prior check');
    else if (!artifactVisibility.has(spec.artifactId)) fail(world, receipt, 'unretained_artifact', 'That prior artifact identifier has not been delivered through eligible memory or source evidence');
    else if (!prior || !prior.receiptId || !prior.hash) fail(world, receipt, 'missing_actual_artifact', 'No successful acquisition receipt establishes that artifact');
    else {
      world.available[spec.artifactId] = spec.artifactId;
      world.reused.push({ artifactId: spec.artifactId, step: world.step, receiptId: receipt.id });
      // No historical text or hidden answer is delivered through reuse.
      receipt.usedArtifactId = spec.artifactId;
      receipt.useAccepted = true;
    }
  } else if (op === 'run' || op === 'observe') {
    if (world.firstSubstantiveStep === null) world.firstSubstantiveStep = world.step;
    const spec = operation(world, action.operation);
    if (!spec) fail(world, receipt, 'unknown_operation', 'Operation is not available in this world');
    else if (world.phase === 'acquisition' && d.fresh.includes(spec)) fail(world, receipt, 'future_operation', 'Current successor work is unavailable during acquisition');
    else if (world.phase === 'action' && (spec === d.original || spec === d.acquisitionFinal)) fail(world, receipt, 'stale_operation', 'The original scoped operation cannot certify the current request');
    else if (world.phase === 'action' && d.fresh.includes(spec) && effectiveIdentity(world) === 'unresolved' && spec.identityDependent) fail(world, receipt, 'identity_unresolved', 'This operation requires established identity and its scoped authorization');
    else if (world.phase === 'action' && d.fresh.includes(spec) && spec.identityDependent && !sourceEligibility.has('src-0011')) fail(world, receipt, 'mapping_not_delivered', 'The applicable authoritative mapping has not been delivered through eligible evidence');
    else if (world.withheld && world._scenario.controlExpectation.operationBlock.includes(action.operation)) fail(world, receipt, 'historical_evidence_unavailable', 'A required historical check or discriminator is not established by eligible evidence');
    else {
      const prerequisites = spec.prerequisites || [];
      const missing = prerequisites.filter(id => !world.available[id]);
      const badFields = (spec.fields || []).filter(field => !equivalentValue(field, action.inputs?.[field], Object.hasOwn(d.knownFields, field) ? d.knownFields[field] : world.questionAnswers[field]));
      const supplied = action.artifacts || [];
      const invalidRef = supplied.some(ref => !artifactReference(world, ref) && !world.reused.some(x => x.artifactId === ref));
      if (missing.length) fail(world, receipt, 'missing_prerequisite', `Required observed artifact receipts are absent: ${missing.join(', ')}`);
      else if (badFields.length) fail(world, receipt, 'missing_or_wrong_input', `Use actual supplied values for: ${badFields.join(', ')}`);
      else if (invalidRef) fail(world, receipt, 'invalid_artifact_reference', 'A supplied artifact reference has no actual receipt in this fork');
      else if (world.phase === 'action' && d.acquisitionChecks.includes(spec)) {
        world.repeated.push({ artifactId: spec.artifactId, operation: spec.operation, step: world.step });
        const repeat = { ...spec, artifactId: `repeated-${spec.artifactId}`, baseArtifactId: spec.artifactId };
        addArtifact(world, receipt, repeat, `Fresh repetition of unchanged check: ${spec.observation}`);
      } else {
        const observation = world.phase === 'acquisition' ? spec.observation : actualCurrentObservation(world, spec, action.inputs || {});
        addArtifact(world, receipt, spec, observation, action.inputs || {}, prerequisites);
      }
    }
  } else if (op === 'finish' || op === 'block') {
    const refs = action.artifacts;
    const invalid = !Array.isArray(refs) || refs.some(ref => !artifactReference(world, ref) && !world.reused.some(x => x.artifactId === (typeof ref === 'string' ? ref : ref?.artifactId)));
    const expected = world.phase === 'acquisition' ? [...d.acquisitionChecks, d.original, d.acquisitionFinal].filter(Boolean) : neededFresh(world).filter(x => !world.withheld || !world._scenario.controlExpectation.operationBlock.includes(x.operation));
    const missing = expected.filter(x => !world.available[x.artifactId]).map(x => x.artifactId);
    const mustBlock = world.phase === 'action' && (effectiveIdentity(world) === 'unresolved' || world.withheld);
    if (invalid) fail(world, receipt, 'invalid_final_reference', 'All output artifact references must identify successful actual receipts');
    else if (!(action.summary || action.reason)?.trim()) fail(world, receipt, 'missing_summary', 'A concise result summary or block reason is required');
    else if (missing.length) fail(world, receipt, 'incomplete_work', `Applicable work is incomplete: ${missing.join(', ')}`);
    else if (mustBlock && op !== 'block') fail(world, receipt, 'unsafe_completion', 'A missing discriminator/evidence condition blocks resolved completion');
    else if (!mustBlock && op === 'block') fail(world, receipt, 'unsupported_block', 'The required evidence and fields can establish completion in this world');
    else {
      world.status = op === 'block' ? 'blocked' : 'complete';
      world.terminal = { status: world.status, step: world.step, summary: action.summary || action.reason, artifacts: clone(refs) };
      receipt.terminal = clone(world.terminal);
    }
  } else fail(world, receipt, 'unknown_action', 'Unsupported action');
  return record();
}

/** Reject dependent same-invocation batches: only pre-batch evidence may justify each operation. */
export function applyBatch(world, actions) {
  if (!Array.isArray(actions)) return [applyAction(world, actions)];
  const batchStart = clone(world);
  return actions.map(action => applyAction(world, action, { batchStart }));
}

export function acquisitionSources(world) {
  if (world.phase !== 'acquisition') throw new Error('Expected acquisition world');
  const occurrences = new Map();
  return world.receipts.filter(r => r.ok && r.artifact).map(r => {
    const occurrence = (occurrences.get(r.operation) || 0) + 1;
    occurrences.set(r.operation, occurrence);
    return { id: `acq-${r.operation}${occurrence > 1 ? `-occurrence-${occurrence}` : ''}`, actor: 'Originating tool', conversation: 'originating-work', text: `${r.artifact.observation}\nArtifact ${r.artifact.id}; applicability: ${r.artifact.applicability}; receipt ${r.id}.` };
  });
}

export function deterministicChecks(scenario, world) {
  const questionFailures = new Set(world.questionEvents.filter(x => ['premature', 'inactive', 'repeated', 'already_known', 'extraneous'].includes(x.status)).map(x => x.field));
  const Q = scenario.qUnits.map(unit => {
    const events = world.questionEvents.filter(x => x.field === unit.field);
    const used = world.actions.filter(x => x.receipt.ok && ((x.action.op === 'run' && Object.hasOwn(x.action.inputs || {}, unit.field || '')) || (stateFor(world).conditional?.dependsOnField === unit.field && x.action.op === 'ask' && x.receipt.answers?.[stateFor(world).conditional.field]?.status === 'answered')));
    let pass;
    if (unit.guard) pass = !world.questionEvents.some(x => ['premature', 'inactive', 'repeated', 'already_known', 'extraneous'].includes(x.status) && !scenario.qUnits.some(u => !u.guard && u.field === x.field));
    else if (unit.kind === 'necessary') pass = events.some(x => x.status === 'answered') && used.length > 0 && !questionFailures.has(unit.field);
    else pass = events.length === 0 && (!Object.hasOwn(stateFor(world).knownFields, unit.field) || used.length > 0);
    return { id: unit.id, kind: unit.kind, status: pass ? 'pass' : 'fail', receiptIds: events.map(x => x.receiptId).concat(used.map(x => x.receipt.id)) };
  });
  const W = scenario.wUnits.map(unit => {
    let pass;
    if (unit.guard) pass = world.repeated.length === 0 && !world.errors.some(x => ['stale_operation', 'identity_unresolved', 'unknown_operation'].includes(x.code));
    else if (unit.kind === 'reuse') pass = world.reused.some(x => x.artifactId === unit.artifactId) && !world.repeated.some(x => x.artifactId === unit.artifactId) && Object.values(world.artifacts).some(x => x.prerequisites?.includes(unit.artifactId));
    else pass = !!world.artifacts[unit.artifactId];
    return { id: unit.id, kind: unit.kind, status: pass ? 'pass' : 'fail', receiptIds: world.receipts.filter(x => x.ok && (x.usedArtifactId === unit.artifactId || x.artifact?.id === unit.artifactId)).map(x => x.id) };
  });
  const D = { observed: !!world.identityDecision || world.outwardStatements.length > 0, classification: world.identityDecision?.classification || null, atStep: world.identityDecision?.atStep || null, firstSubstantiveStep: world.firstSubstantiveStep, late: !!world.identityDecision && world.firstSubstantiveStep !== null && world.identityDecision.atStep >= world.firstSubstantiveStep, statements: clone(world.outwardStatements), semanticStatus: 'pending' };
  return { Q, W, D, terminal: clone(world.terminal), complete: ['complete', 'blocked'].includes(world.status) && world.errors.length === 0, resolvedWork: world.status === 'complete' && world.errors.length === 0, correctlyBlocked: world.status === 'blocked' && world.errors.length === 0, errors: clone(world.errors) };
}

/** Controller-only offline witnesses. Never deliver these plans to evaluated agents. */
export function validAcquisitionPlan(scenario) {
  const d = scenario.worldDefinition;
  return [...d.acquisitionChecks.map(x => ({ op: 'run', operation: x.operation })), { op: 'run', operation: d.original.operation }, ...(d.acquisitionFinal ? [{ op: 'run', operation: d.acquisitionFinal.operation }] : []), { op: 'finish', summary: 'Offline witness handoff; not an evaluated originating answer.', artifacts: [...d.acquisitionChecks.map(x => x.artifactId), d.original.artifactId, ...(d.acquisitionFinal ? [d.acquisitionFinal.artifactId] : [])] }];
}

export function validActionPlan(scenario, { withheld = false } = {}) {
  const d = scenario.worldDefinition;
  const effective = withheld && scenario.withheldSourceIds.includes('src-0011') ? 'unresolved' : scenario.identity;
  const fresh = d.fresh.filter(x => effective !== 'unresolved' || !x.identityDependent).filter(x => !withheld || !scenario.controlExpectation.operationBlock.includes(x.operation));
  const needed = new Set(fresh.flatMap(x => x.prerequisites));
  const reuse = d.reuse.filter(x => needed.has(x.artifactId)).filter(x => !withheld || !scenario.controlExpectation.blockedReuse?.includes(x.artifactId));
  const fields = [...new Set(fresh.flatMap(x => x.fields))].filter(f => Object.hasOwn(d.unknownFields, f));
  if (d.conditional && fields.includes(d.conditional.field) && !fields.includes(d.conditional.dependsOnField)) fields.push(d.conditional.dependsOnField);
  const initial = fields.filter(f => f !== d.conditional?.field);
  const conditional = fields.filter(f => f === d.conditional?.field);
  const plan = [{ op: 'lookup', query: scenario.family }];
  const message = `The authoritative mapping establishes ${effective}; ${effective === 'unresolved' ? 'the missing discriminator prevents an identity link' : 'this identifies the underlying scoped target, not just matching wording or values'}. Evidence: src-0011.`;
  if (initial.length) plan.push({ op: 'ask', fields: initial, message });
  else plan.push({ op: 'message', text: message });
  if (conditional.length) plan.push({ op: 'ask', fields: conditional });
  for (const x of reuse) plan.push({ op: 'reuse', artifactId: x.artifactId });
  if (withheld) for (const name of scenario.controlExpectation.recoverableBy || []) plan.push({ op: 'run', operation: name });
  for (const x of fresh) plan.push({ op: 'run', operation: x.operation, inputs: Object.fromEntries(x.fields.map(f => [f, Object.hasOwn(d.knownFields, f) ? d.knownFields[f] : d.unknownFields[f]])) });
  const blocked = effective === 'unresolved' || withheld;
  plan.push({ op: blocked ? 'block' : 'finish', ...(blocked ? { reason: 'Missing historical discriminator or check; independent work is complete and dependent execution remains blocked.' } : { summary: 'Applicable current work completed with actual receipts and limited authorization.' }), artifacts: fresh.map(x => x.artifactId).concat(reuse.map(x => x.artifactId)) });
  return plan;
}

export function alternateActionPlan(scenario, options = {}) {
  const plan = validActionPlan(scenario, options);
  const reuse = plan.filter(x => x.op === 'reuse').reverse();
  let cursor = 0;
  return plan.map(x => x.op === 'reuse' ? reuse[cursor++] : x);
}
