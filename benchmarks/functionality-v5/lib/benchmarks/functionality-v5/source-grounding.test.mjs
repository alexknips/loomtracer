import test from 'node:test';
import assert from 'node:assert/strict';
import { buildCorpus, bindAcquisition, eligibleSources } from './corpus.mjs';
import { validateCorpus, validateProducerEvidence } from './validation.mjs';
import * as worldAdapter from './world.mjs';
import { controlSources } from './run-screen.mjs';

const declaration = 'I am the producer for Aurora locale validation.';
const releases = () => buildCorpus().filter(s => s.family === 'release');
const origin = s => s.episodes[0].sources.find(x => x.id === 'src-0001');

test('all nine release paths explicitly ground the required producer before maintenance', () => {
  for (const s of releases()) {
    const before = structuredClone(s);
    const result = validateProducerEvidence(s);
    assert.equal(result.passed, true, `${s.id}: ${result.errors}`);
    assert.equal(origin(s).actor, s.worldDefinition.knownFields.producer);
    assert.ok(origin(s).text.includes(declaration));
    assert.deepEqual(s, before, 'Validation must not inject a missing source or edit the key');
  }
});

test('role absence and misleading role text fail even when the known value is unchanged', () => {
  const invalid = [
    '', 'I selected the Tuesday window for Aurora.', 'Priya is mentioned in the production conversation.',
    'I am not the producer for Aurora locale validation.', 'I may be the producer for Aurora locale validation.',
    'I am the coordinator for Aurora locale validation.', 'I am the producer for Birch locale validation.',
    'A draft says "I am the producer for Aurora locale validation."',
    `${declaration} Tomas is the producer for Aurora locale validation.`,
  ];
  for (const s of releases()) for (const replacement of invalid) {
    const changed = structuredClone(s);
    origin(changed).text = origin(changed).text.replace(declaration, replacement);
    assert.equal(validateProducerEvidence(changed).passed, false, `${s.id}: ${replacement}`);
  }
});

test('wrong actor or private-key attribution fails across release difficulty and identity paths', () => {
  for (const s of releases()) for (const mutate of [
    s => { origin(s).actor = 'Tomas'; },
    s => { s.worldDefinition.knownFields.producer = 'Tomas'; },
    s => { origin(s).conversation = 'unrelated-production'; },
    s => { delete s.worldDefinition.knownFieldEvidence; },
    s => { s.worldDefinition.knownFieldEvidence.producer.basis = 'quoted-substring'; },
  ]) {
    const changed = structuredClone(s); mutate(changed);
    assert.equal(validateProducerEvidence(changed).passed, false, s.id);
  }
});

test('later or unrelated sources cannot silently repair missing historical producer evidence', () => {
  for (const s of releases()) for (const target of ['src-0006', 'noise-2-001', 'future-producer']) {
    const changed = structuredClone(s), source = origin(changed);
    source.text = source.text.replace(declaration, '');
    const other = changed.episodes.flatMap(e => e.sources).find(x => x.id === target);
    if (other) Object.assign(other, { actor: 'Priya', conversation: 'production', text: declaration });
    else changed.episodes.push({ number: 5, sources: [{ id: target, actor: 'Priya', conversation: 'production', text: declaration }] });
    changed.worldDefinition.knownFieldEvidence.producer.sourceId = target;
    assert.equal(validateProducerEvidence(changed).passed, false, `${s.id}: ${target}`);
  }
});

test('full corpus validation rejects a missing producer source despite executable key-driven plans', () => {
  const corpus = buildCorpus();
  const s = corpus.find(x => x.id === 'v5-release-demanding-unresolved');
  origin(s).text = origin(s).text.replace(declaration, '');
  const result = validateCorpus(corpus, { worldAdapter });
  assert.equal(result.passed, false);
  assert.ok(result.errors.some(e => e === `${s.id}: Producer requirement is grounded in an explicit eligible role declaration`));
});

test('release source controls retain grounded producer while withholding only declared decisive evidence', () => {
  for (const s of releases()) {
    const acquisition = worldAdapter.createWorld(s, { phase: 'acquisition' });
    for (const action of worldAdapter.validAcquisitionPlan(s)) worldAdapter.applyAction(acquisition, action);
    const bound = bindAcquisition(s, acquisition, { summary: 'Original checks complete.' });
    for (const control of ['oracle', 'withheld']) for (const kind of ['H', 'action']) {
      const selected = controlSources(bound.scenario, { kind, control });
      const producer = selected.find(x => x.id === 'src-0001');
      assert.ok(producer?.text.includes(declaration), `${s.id}/${control}/${kind}: grounded producer retained`);
      assert.equal(producer.actor, 'Priya');
      if (control === 'withheld') for (const id of s.withheldSourceIds) assert.ok(!selected.some(x => x.id === id));
    }
    const withheld = eligibleSources(bound.scenario, { withheld: true });
    if (s.identity === 'unresolved') {
      assert.ok(!withheld.some(x => x.id === 'acq-validate_license'));
      assert.ok(withheld.some(x => x.id === 'src-0011'), 'Genuine unresolved identity stays visible');
    }
  }
});
