import test from 'node:test';
import assert from 'node:assert/strict';
import { nativeMaintenancePrompt, nativeEnvironment, inspectNativeExecutable, nativeDeliveredArtifactText } from './native.mjs';

test('native maintenance prompt contains only previous extracted memory and the new episode', () => {
  const previous = { id: 'episode-1', sourceCutoff: 4, records: [{ id: 'decision-one', sourceIds: ['src-1'] }], projection: [{ id: 'decision-one', native: { title: 'Saved extraction' } }],
    db: { directory: '/private/secret/ledger' }, sourceHashes: { 'future-secret': 'hidden' }, implementation: { executable: '/secret/native' } };
  const episode = { number: 2, hiddenAnswer: 'TOP-SECRET-ANSWER', future: 'TOP-SECRET-FUTURE', sources: [{ id: 'src-5', text: 'A new unrelated message.', hiddenAnswer: 'SOURCE-HIDDEN-ANSWER', sourceSpan: { start: 0, end: 3, secret: 'SPAN-HIDDEN-ANSWER' } }] };
  const prompt = nativeMaintenancePrompt(previous, episode);
  assert.match(prompt, /Saved extraction/); assert.match(prompt, /A new unrelated message/);
  assert.doesNotMatch(prompt, /secret\/ledger|future-secret|secret\/native|TOP-SECRET|SOURCE-HIDDEN|SPAN-HIDDEN/);
  assert.match(prompt, /An extraction correction is not a participant reversal/);
  assert.match(prompt, /sourceIds/); assert.match(prompt, /ground_decision/);
});

test('native subprocess environment exposes no remote backend, provider credentials or implicit project settings', () => {
  const env = nativeEnvironment();
  assert.deepEqual(Object.keys(env).sort(), ['LANG', 'LC_ALL', 'PATH', 'RUST_LOG']);
  assert.ok(!Object.keys(env).some(k => /HIVEMIND_|OPENAI_|API|HOME|TOKEN|KEY/.test(k)));
});

test('native executable inspection refuses relative and missing executables without provider calls', async () => {
  await assert.rejects(inspectNativeExecutable({ executable: 'hivemind' }), /Absolute/);
  await assert.rejects(inspectNativeExecutable({ executable: '/no-such-v5-native-binary' }), /ENOENT/);
});


test('native maintenance prompt applies bounded exact source delivery and refuses ambiguous metadata', () => {
  assert.throws(() => nativeMaintenancePrompt(null, [{ id: 'one', text: 'x'.repeat(65536) }]), /exceeds/);
  assert.throws(() => nativeMaintenancePrompt(null, [{ id: 'one', text: 'Valid', actor: { hidden: 'answer' } }]), /must be text/);
  assert.throws(() => nativeMaintenancePrompt(null, [{ id: 'one', text: 'First' }, { id: 'one', text: 'Second' }]), /Duplicate/);
  const prompt = nativeMaintenancePrompt(null, { messages: [{ id: 'visible', text: 'Exact new words.', actor: 'Ada', order: 4 }], hiddenAnswer: 'hidden-criterion' });
  assert.match(prompt, /Exact new words/); assert.match(prompt, /"order":4/); assert.doesNotMatch(prompt, /hidden-criterion/);
});


test('native artifact eligibility reads actual returned record content, never query echoes or projections', () => {
  const records = [{ id: 'decision-retained', kind: 'decision', key: 'artifact-key-is-not-evidence' }];
  const payload = { native: { data: { query: 'artifact-query-echo', digest: { summary: 'artifact-digest-metadata' }, ranked: { items: [
    { decision: { id: 'decision-retained', rationale: 'artifact-rationale was completed.' }, missing_terms: ['artifact-missing-term'], snippets: [
      { field: 'evidence.content', value: 'artifact-evidence was verified.' }, { field: 'decision.id', value: 'artifact-id-field' }, { field: 'actor.source_ref', value: 'artifact-actor-field' } ] },
    { decision: { id: 'decision-not-retained', rationale: 'artifact-foreign-record' } },
  ] } } }, retainedReferences: [{ key: 'artifact-sidecar-key' }] };
  const text = nativeDeliveredArtifactText(payload, records);
  assert.match(text, /artifact-rationale/); assert.match(text, /artifact-evidence/);
  assert.doesNotMatch(text, /artifact-query|artifact-digest|artifact-missing|artifact-id-field|artifact-actor|artifact-foreign|artifact-key|artifact-sidecar/);
  assert.equal(nativeDeliveredArtifactText({ native: { data: { query: 'artifact-only-echo', ranked: { items: [] } } } }, records), '');
  const neighborhood = { native: { data: { root: { id: 'decision-retained', rationale: 'artifact-root' }, nodes: [
    { id: 'evidence-inline', kind: 'evidence', label: 'artifact-inline-evidence' }, { id: 'agent:artifact-agent-id', kind: 'actor', label: 'artifact-actor-label' },
  ] } } };
  assert.match(nativeDeliveredArtifactText(neighborhood, records), /artifact-inline-evidence/);
  assert.doesNotMatch(nativeDeliveredArtifactText(neighborhood, records), /artifact-agent|artifact-actor/);
});
