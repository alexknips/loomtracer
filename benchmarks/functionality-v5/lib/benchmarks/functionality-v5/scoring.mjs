/** Strict semantic units remain binary; granularity changes aggregation, not correctness. */
export const SCORING_VERSION = 'functionality-v5-strict-units-1.0.1';
export const REPORT_VERSION = 'functionality-v5-report-1.0.2';
export const DIMENSIONS = ['premises', 'relationships', 'attribution', 'changes'];
export const LEVELS = ['easy', 'intermediate', 'demanding'];
export const IDENTITY_CLASSES = ['same', 'distinct', 'unresolved'];
const clone = value => structuredClone(value);
const unique = values => new Set(values).size === values.length;
const mean = values => values.length && values.every(x => Number.isFinite(x)) ? values.reduce((a, b) => a + b, 0) / values.length : null;
const strict = j => !j || ['pending', 'uncertain', 'disputed'].includes(j.status ?? j.verdict) ? 'pending'
  : j.incomplete || j.ambiguous || j.materialContradiction || j.supported === false || j.completeUnambiguous === false
    ? 'fail' : ['pass', 'fail'].includes(j.status ?? j.verdict) ? (j.status ?? j.verdict) : 'pending';
export function harmonic(a, b) { return a === null || b === null ? null : a === 0 || b === 0 ? 0 : 2 * a * b / (a + b); }
export function tally(statuses = []) {
  const planned = statuses.length, pass = statuses.filter(s => s === 'pass').length,
    fail = statuses.filter(s => s === 'fail').length, pending = planned - pass - fail;
  return { planned, pass, fail, pending, lower: planned ? pass / planned : null, upper: planned ? (pass + pending) / planned : null };
}
const rating = (lower, upper, extra = {}) => ({ ...extra, lower: lower === null ? null : 100 * lower,
  upper: upper === null ? null : 100 * upper, rating: lower !== null && lower === upper ? Math.round(100 * lower) : null,
  raw: lower !== null && lower === upper ? 100 * lower : null,
  status: lower === null || upper === null ? 'unavailable' : lower === upper ? 'model-reviewed-pending-human-audit' : 'pending' });

export function scoreH(units, judgments = [], { unsupportedClaims = [], humanAudit = 'pending' } = {}) {
  if (!Array.isArray(units) || !unique(units.map(u => u.id)) || !unique(judgments.map(j => j.id))) throw new Error('Unique H unit identifiers required');
  if (units.some(u => !DIMENSIONS.includes(u.dimension))) throw new Error('Unknown H dimension');
  if (judgments.some(j => !units.some(u => u.id === j.id))) throw new Error('Unexpected H judgment');
  const rows = units.map(u => ({ ...clone(u), status: strict(judgments.find(j => j.id === u.id)), invalidatedBy: [] }));
  // A missing statement is not a contradictory premise. Only established material
  // errors propagate through the frozen semantic-dependency edges.
  const contradictory = new Set(judgments.filter(j => j.materialContradiction && strict(j) === 'fail').map(j => j.id));
  for (const claim of unsupportedClaims.filter(c => c.material !== false)) {
    for (const id of claim.affectedUnitIds ?? []) {
      if (!units.some(u => u.id === id)) throw new Error('Unsupported claim targets an unknown H unit');
      contradictory.add(id);
    }
  }
  let changed = true;
  while (changed) {
    changed = false;
    for (const row of rows) {
      const causes = (row.dependsOn ?? []).filter(id => contradictory.has(id));
      if ((contradictory.has(row.id) || causes.length) && row.status !== 'fail') row.status = 'fail';
      row.invalidatedBy = causes;
      if (causes.length && !contradictory.has(row.id)) { contradictory.add(row.id); changed = true; }
    }
  }
  const dimensions = Object.fromEntries(DIMENSIONS.map(d => [d, tally(rows.filter(r => r.dimension === d).map(r => r.status))]));
  const counts = tally(rows.map(r => r.status));
  const materialClaims = unsupportedClaims.filter(c => c.material !== false);
  const completeChain = counts.fail || materialClaims.length ? 'fail' : counts.pending || !counts.planned ? 'pending' : 'pass';
  return { version: SCORING_VERSION, ...rating(mean(Object.values(dimensions).map(d => d.lower)), mean(Object.values(dimensions).map(d => d.upper))),
    units: rows, counts, dimensions, completeChain, unsupportedClaims: clone(unsupportedClaims), humanAudit,
    semanticSupport: 'Eligible source meaning, independent of participant citation syntax.' };
}

function scoreBalanced(units, judgments, sides, deterministic = []) {
  if (!unique(units.map(u => u.id)) || !unique(judgments.map(j => j.id))) throw new Error('Unique behavior obligation identifiers required');
  if (judgments.some(j => !units.some(u => u.id === j.id))) throw new Error('Unexpected behavior judgment');
  const rows = units.map(u => {
    let status = strict(judgments.find(j => j.id === u.id));
    if (deterministic.find(d => d.id === u.id)?.status === 'fail') status = 'fail';
    return { ...clone(u), status };
  });
  const rates = Object.fromEntries(sides.map(side => [side, tally(rows.filter(r => r.kind === side).map(r => r.status))]));
  return { ...rating(harmonic(rates[sides[0]].lower, rates[sides[1]].lower), harmonic(rates[sides[0]].upper, rates[sides[1]].upper)),
    units: rows, rates, counts: tally(rows.map(r => r.status)), guards: rows.filter(r => r.guard || /guard/.test(r.id)) };
}

export function scoreAction(scenario, grade = {}, { deterministic = grade.deterministic ?? {}, humanAudit = 'pending', mode = 'ordinary' } = {}) {
  const Q = scoreBalanced(scenario.qUnits ?? [], grade.qUnits ?? grade.questions ?? [], ['necessary', 'avoid'], deterministic.Q ?? []);
  const W = scoreBalanced(scenario.wUnits ?? [], grade.wUnits ?? grade.work ?? [], ['reuse', 'fresh'], deterministic.W ?? []);
  const identity = grade.identity ?? grade.D;
  let status = strict(identity);
  const dt = deterministic.D ?? {};
  if (dt.late || identity?.late || dt.observed === false || identity?.observed === false) status = 'fail';
  const D = { status, expected: scenario.identityEvidence?.classification ?? scenario.identity,
    observed: dt.observed ?? identity?.observed ?? null, late: Boolean(dt.late || identity?.late),
    category: identity?.category ?? (dt.late || identity?.late ? 'late' : dt.observed === false ? 'unobserved' : null),
    judgment: clone(identity ?? null) };
  const declared = grade.completion?.status ?? grade.completeTask;
  const materialClaims = (grade.unsupportedClaims ?? []).some(c => c.material !== false);
  const completeTask = deterministic.complete === false || declared === 'fail' || Q.counts.fail || W.counts.fail || materialClaims || (mode === 'prompted' && D.status === 'fail') ? 'fail'
    : declared === 'pass' && Q.counts.pending === 0 && W.counts.pending === 0 && (mode !== 'prompted' || D.status === 'pass') ? 'pass' : 'pending';
  return { version: SCORING_VERSION, Q, W, D, completeTask, terminal: deterministic.terminal ?? grade.terminal ?? null,
    humanAudit, unsupportedClaims: clone(grade.unsupportedClaims ?? []) };
}

/** Source-citation syntax and existence are an independent diagnostic, never an H veto. */
export function citationCompliance(citations = [], registry = [], { cutoff = Infinity } = {}) {
  const byId = new Map(registry.map(r => [r.id, r]));
  const rows = citations.map(citation => {
    const id = typeof citation === 'string' ? citation : citation.sourceId ?? citation.id;
    const target = byId.get(id);
    let error = !id ? 'invalid-shape' : !target ? 'missing-reference' : (target.order ?? target.sourceOrder ?? 0) > cutoff ? 'future-reference' : null;
    if (!error && typeof citation === 'object' && citation.startLine !== undefined) {
      const length = String(target.text ?? target.content ?? '').split('\n').length;
      if (!Number.isInteger(citation.startLine) || !Number.isInteger(citation.endLine) || citation.startLine < 1 || citation.endLine < citation.startLine || citation.endLine > length) error = 'invalid-line-range';
    }
    return { citation: clone(citation), status: error ? 'fail' : 'pass', error };
  });
  const valid = rows.filter(r => r.status === 'pass').length;
  return { emitted: rows.length, valid, invalid: rows.length - valid, rating: rows.length ? 100 * valid / rows.length : null, rows,
    note: 'No emitted citations gives unavailable compliance, not semantic failure or perfect compliance.' };
}

const idOf = value => value.scenarioId ?? value.caseId ?? value.id;
function resultGrade(row) { return row?.grade?.value ?? row?.grade?.grade ?? row?.grade ?? row?.review ?? {}; }

export function ratings({ scenarios, results = [], arm = 'stock-maintained-notes', mode = 'ordinary' }) {
  if (!['ordinary', 'prompted'].includes(mode) || !scenarios?.length || !unique(scenarios.map(s => s.id))) throw new Error('Explicit mode and unique planned scenarios required');
  const filtered = results.filter(r => (!r.arm || r.arm === arm) && (!r.control || r.control === 'primary'));
  const keys = filtered.map(r => `${idOf(r)}:${r.kind}:${r.kind === 'H' ? 'prompted' : r.mode}`);
  if (!unique(keys)) throw new Error('Duplicate result for planned fork');
  const cases = scenarios.map(s => {
    const h = filtered.find(r => idOf(r) === s.id && r.kind === 'H');
    const a = filtered.find(r => idOf(r) === s.id && r.kind === 'action' && r.mode === mode);
    const hg = resultGrade(h), ag = resultGrade(a);
    return { id: s.id, level: s.level, family: s.family, identity: s.identityEvidence?.classification,
      H: scoreH(s.hUnits ?? [], hg.hUnits ?? hg.units ?? [], { unsupportedClaims: hg.unsupportedClaims ?? [], humanAudit: h?.humanAudit ?? 'pending' }),
      ...scoreAction(s, ag, { deterministic: a?.deterministic ?? ag.deterministic ?? {}, humanAudit: a?.humanAudit ?? 'pending', mode }),
      execution: { H: h?.status ?? 'not-run', action: a?.status ?? 'not-run' } };
  });
  const perLevel = Object.fromEntries(LEVELS.map(level => {
    const rows = cases.filter(c => c.level === level);
    const H = rating(mean(rows.map(r => r.H.lower === null ? null : r.H.lower / 100)), mean(rows.map(r => r.H.upper === null ? null : r.H.upper / 100)), { plannedCases: rows.length });
    const balanced = (component, sides) => {
      const rates = Object.fromEntries(sides.map(side => [side, {
        lower: mean(rows.map(r => r[component].rates[side].lower)), upper: mean(rows.map(r => r[component].rates[side].upper)),
        denominator: rows.reduce((n, r) => n + r[component].rates[side].planned, 0),
        pass: rows.reduce((n, r) => n + r[component].rates[side].pass, 0),
        pending: rows.reduce((n, r) => n + r[component].rates[side].pending, 0) }]));
      return rating(harmonic(rates[sides[0]].lower, rates[sides[1]].lower), harmonic(rates[sides[0]].upper, rates[sides[1]].upper), { rates, plannedCases: rows.length });
    };
    const classes = Object.fromEntries(IDENTITY_CLASSES.map(identity => [identity, tally(rows.filter(r => r.identity === identity).map(r => r.D.status))]));
    const D = rating(mean(Object.values(classes).map(r => r.lower)), mean(Object.values(classes).map(r => r.upper)), { classes, plannedCases: rows.length });
    const components = { H, Q: balanced('Q', ['necessary', 'avoid']), W: balanced('W', ['reuse', 'fresh']), D };
    const overallBound = bound => Object.values(components).every(c => c[bound] !== null)
      ? (components.H[bound] + (components.Q[bound] + components.W[bound]) / 2 + components.D[bound]) / 300 : null;
    return [level, { components, overall: rating(overallBound('lower'), overallBound('upper')), plannedCases: rows.length }];
  }));
  const components = Object.fromEntries(['H', 'Q', 'W', 'D'].map(component => [component,
    rating(mean(LEVELS.map(l => perLevel[l].components[component].lower === null ? null : perLevel[l].components[component].lower / 100)),
      mean(LEVELS.map(l => perLevel[l].components[component].upper === null ? null : perLevel[l].components[component].upper / 100)))]));
  const overallBound = bound => Object.values(components).every(c => c[bound] !== null)
    ? (components.H[bound] + (components.Q[bound] + components.W[bound]) / 2 + components.D[bound]) / 300 : null;
  return { version: REPORT_VERSION, scoringVersion: SCORING_VERSION, arm, mode, plannedCases: scenarios.length, cases, perLevel, components,
    overall: rating(overallBound('lower'), overallBound('upper')), humanAudit: 'pending',
    interpretation: 'Strict binary local requirements with equal dimensions and cases. Q/W balance opposing decisions; D balances identity classes. Missing strata or bands prevent an aggregate. Ordinary and prompted modes remain separate.' };
}

/** Evidence-supported loss labels only. Unknown stage evidence remains not established. */
export function firstObservedLoss({ acquired, retained, delivered, reconstructed, applied }) {
  const steps = [['acquisition', acquired], ['maintenance/capture', retained], ['delivery/lookup', delivered], ['answer reconstruction', reconstructed], ['action application', applied]];
  for (let i = 0; i < steps.length; i++) {
    const [stage, evidence] = steps[i];
    if (evidence?.status === 'absent' && evidence.evidence?.length && steps.slice(0, i).every(([, e]) => e?.status === 'present' && e.evidence?.length)) return { stage, evidence: clone(evidence.evidence) };
    if (!evidence || evidence.status === 'unknown') return { stage: 'not established', evidence: [] };
  }
  return { stage: 'not established', evidence: [] };
}
