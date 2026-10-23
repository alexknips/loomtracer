/** The words for what a bundle says: Loomtracer's relations read as the graph contract's arrow labels (docs/GRAPH_CONTRACT.md). */
const LOOMTRACER_WORDS: Record<string, string> = {
  'legacy:has_option': 'weighs', 'legacy:chose': 'chose', 'legacy:based_on': 'based on',
  'legacy:premised_on': 'rests on', 'legacy:premised_on_direct': 'rests on',
  'legacy:supports': 'supports', 'legacy:refutes': 'refutes', 'legacy:supersedes': 'supersedes',
  'legacy:same_as': 'is the same as', 'legacy:follows_from': 'follows from', 'legacy:answers': 'answers',
};

export function relationLabel(relation: string): string {
  return LOOMTRACER_WORDS[relation] ?? relation
    .replace(/^legacy-type:/, 'Legacy ').replace(/^legacy-status:/, 'Stored status: ').replace(/^legacy:/, 'Legacy: ')
    .replace(/^status:/, 'Status: ').replaceAll('_', ' ');
}

/** Contested, superseded and the rest are told apart by colour as well as by word. */
export const statusTone = (label: string) => label.startsWith('status:') ? `status status-${label.slice(7)}` : '';
