import type { Connection, Entry, Provenance, ReasoningBundle, SourceMessage } from '@reasoning';

/** Intentionally fictional; this file contains no private conversation material. */
export function makeDemoBundle(): ReasoningBundle {
  const contexts: ReasoningBundle['contexts'] = [{ id: 'demo-context', title: 'Illustration: choosing an option', kind: 'illustration' }];
  const times = ['2026-10-05T09:00:00-07:00', '2026-10-05T09:02:00-07:00', '2026-10-06T10:00:00-07:00', '2026-10-09T11:00:00-07:00'];
  const texts = [
    'Which option should we choose: option A or option B? We need to meet the performance requirement while minimizing cost.',
    'Both options meet the performance requirement in our small-workload test. A costs less than B. I recommend option A because both pass and A costs less.',
    "Let's proceed with option A, adopting the recommendation above.",
    'The new load test shows A fails the performance requirement at our expected traffic. The earlier recommendation relied on a smaller workload. This challenges the basis for choosing A; no replacement choice has been made.',
  ];
  const sources: SourceMessage[] = texts.map((content, index) => ({ id: `S${index + 1}`, originalId: `fictional-message-${index + 1}`, actorId: [0, 2].includes(index) ? 'fictional-human' : 'fictional-agent', role: [0, 2].includes(index) ? 'user' : 'assistant', content, occurredAt: times[index], loggedAt: times[index], order: index + 1, sourceRef: `synthetic://walkthrough/message-${index + 1}`, attachmentIds: [], active: true }));
  const provenance = (sourceIndex: number, excerpt: string): Provenance => {
    const source = sources[sourceIndex]; const start = source.content.indexOf(excerpt);
    if (start < 0) throw new Error('Invalid demo excerpt');
    return { expressedBy: source.actorId, recordedBy: 'demo-recorder', occurredAt: source.occurredAt, recordedAt: '2026-10-12T09:00:00-07:00', basis: 'direct', sourceSpans: [{ messageId: source.id, start, end: start + excerpt.length }] };
  };
  const entry = (id: string, sourceIndex: number, excerpt: string, labels: string[]): Entry => ({ id: `${id}:v1`, logicalId: id, version: 1, contextId: 'demo-context', labels, provenance: provenance(sourceIndex, excerpt) });
  const entries = [
    entry('E-question', 0, 'Which option should we choose: option A or option B?', ['question']),
    entry('E-option-a', 0, 'option A', ['option']), entry('E-option-b', 0, 'option B', ['option']),
    entry('E-criterion', 0, 'We need to meet the performance requirement while minimizing cost.', ['criterion']),
    entry('E-test', 1, 'Both options meet the performance requirement in our small-workload test.', ['observation']),
    entry('E-cost', 1, 'A costs less than B.', ['observation']),
    entry('E-recommendation', 1, 'I recommend option A because both pass and A costs less.', ['assessment', 'proposal']),
    entry('E-commitment', 2, sources[2].content, ['commitment']),
    entry('E-new-test', 3, 'The new load test shows A fails the performance requirement at our expected traffic.', ['observation']),
    entry('E-objection', 3, 'The earlier recommendation relied on a smaller workload. This challenges the basis for choosing A; no replacement choice has been made.', ['challenge']),
  ];
  const connect = (id: string, relation: Connection['relation'], participants: [string, string][], sourceIndex: number, excerpt: string): Connection => ({ id: `${id}:v1`, logicalId: id, version: 1, contextId: 'demo-context', relation, participants: participants.map(([entryId, role]) => ({ entryId: `${entryId}:v1`, role })), provenance: provenance(sourceIndex, excerpt) });
  const connections = [
    connect('C-answer-a', 'answers', [['E-option-a', 'answer'], ['E-question', 'question']], 0, sources[0].content),
    connect('C-answer-b', 'answers', [['E-option-b', 'answer'], ['E-question', 'question']], 0, sources[0].content),
    connect('C-support', 'supports', [['E-test', 'evidence'], ['E-cost', 'premise'], ['E-criterion', 'criterion'], ['E-recommendation', 'conclusion']], 1, sources[1].content),
    connect('C-comparison', 'compares', [['E-recommendation', 'assessment'], ['E-option-a', 'alternative'], ['E-option-b', 'alternative'], ['E-criterion', 'criterion']], 1, sources[1].content),
    connect('C-adoption', 'adopts', [['E-commitment', 'commitment'], ['E-recommendation', 'proposal']], 2, sources[2].content),
    connect('C-new-support', 'supports', [['E-new-test', 'evidence'], ['E-objection', 'conclusion']], 3, sources[3].content),
    connect('C-challenge', 'challenges', [['E-objection', 'objection'], ['E-test', 'target']], 3, sources[3].content),
  ];
  return {
    schemaVersion: '0.3.0', namespace: { datasetId: 'synthetic-walkthrough', workspaceId: null, projectId: null }, authorityGrants: [], commitmentReceipts: [], externalEntryRefs: [], id: 'synthetic-walkthrough', title: 'Synthetic walkthrough',
    actors: [{ id: 'fictional-human', name: 'Morgan (illustrative human)', kind: 'human' }, { id: 'fictional-agent', name: 'Agent (illustrative)', kind: 'agent' }, { id: 'demo-recorder', name: 'Example recorder', kind: 'agent' }], contexts, sources, attachments: [],
    manifest: { threadId: 'synthetic-thread', frozenAt: '2026-10-12T09:00:00-07:00', cutoffDescription: 'Four invented contributions, through Friday. No private user data.', segments: [{ ref: 'synthetic://walkthrough', sha256: '0'.repeat(64), selectedMessageIds: sources.map(source => source.id) }], exclusions: ['This scenario is an illustration, not an actual project decision.'] },
    entries, connections,
    runs: [
      { id: 'R1', parentId: null, createdAt: '2026-10-12T09:00:00-07:00', recordedBy: 'demo-recorder', sourceCutoff: 3, sourceMessageIds: ['S1', 'S2', 'S3'], addedEntryIds: entries.slice(0, 8).map(item => item.id), addedConnectionIds: connections.slice(0, 5).map(item => item.id), replacements: [], uncertainties: ['Whether the initial small-workload test represented expected traffic was not established at the time.'], notes: 'Initial extraction through the illustrative Tuesday commitment.', reviewResolutions: [] },
      { id: 'R2', parentId: 'R1', createdAt: '2026-10-12T09:10:00-07:00', recordedBy: 'demo-recorder', sourceCutoff: 4, sourceMessageIds: ['S4'], addedEntryIds: entries.slice(8).map(item => item.id), addedConnectionIds: connections.slice(5).map(item => item.id), replacements: [], uncertainties: ['No revised commitment or withdrawal is recorded after the challenge.'], notes: 'Incremental capture of the illustrative Friday test and objection.', reviewResolutions: [] },
    ], reviewNotes: [],
  };
}
