import type { Connection, Entry, ReasoningBundle, NativeRelation, View } from './types';

export const ENTRY_SIZE = { width: 300, height: 168 };
export const CONNECTION_SIZE = { width: 172, height: 64 };
const LANE_WIDTH = 352;
const ROW_GAP = 62;
const BAND_GAP = 90;

export type GraphRole = { role: string; connectionIds: string[] };
export type GraphNode = {
  id: string; kind: 'entry' | 'connection'; position: { x: number; y: number };
  width: number; height: number; roles: GraphRole[];
  timestamp: string | null; sourceOrder: number | null; cycle: boolean;
};
export type GraphLink = {
  id: string; source: string; target: string; connectionId: string; role: string;
  // A backwards line is retained rather than changing the historical position of an entry.
  pointsToEarlierEntry: boolean;
};
export type GraphModel = { nodes: GraphNode[]; edges: GraphLink[]; warnings: string[] };

const outputRoles: Record<NativeRelation, string> = {
  responds_to: 'response', answers: 'answer', supports: 'conclusion', challenges: 'objection',
  compares: 'assessment', adopts: 'commitment', revises: 'revision', withdraws: 'withdrawal',
};

/** Roles determine direction; a message's timestamp cannot tell us which participant is an answer. */
export function relationLinks(connection: Connection): GraphLink[] {
  const participants = [...connection.participants, ...(connection.rationaleEntryId ? [{ entryId: connection.rationaleEntryId, role: 'rationale' }] : [])];
  return participants.map((participant, index) => {
    const output = participant.role === (connection.relation.startsWith('legacy:') ? 'target' : outputRoles[connection.relation as NativeRelation]);
    return {
      id: `${connection.id}:${index}`, source: output ? connection.id : participant.entryId,
      target: output ? participant.entryId : connection.id, connectionId: connection.id,
      role: participant.role, pointsToEarlierEntry: false,
    };
  });
}

const validTime = (value: string | null) => value && Number.isFinite(Date.parse(value)) ? Date.parse(value) : null;
type Moment = { timestamp: string | null; sourceOrder: number | null; offset: number; time: number; band: string };

/** Undated messages keep their recorded position between dated neighbours; no timestamp is invented. */
function moments(bundle: ReasoningBundle, entries: Entry[]): Map<string, Moment> {
  const sources = [...bundle.sources].sort((a, b) => a.order - b.order);
  const sourceMap = new Map(sources.map(source => [source.id, source]));
  const datedSources = sources.filter(source => validTime(source.occurredAt) !== null);
  const sourceFallback = (order: number) => {
    const before = datedSources.filter(source => source.order < order).at(-1);
    const after = datedSources.find(source => source.order > order);
    const earlier = before ? validTime(before.occurredAt)! : null;
    const later = after ? validTime(after.occurredAt)! : null;
    if (earlier !== null && later !== null && later >= earlier) return earlier + (later - earlier) * (order - before!.order) / (after!.order - before!.order);
    if (earlier !== null) return earlier + (order - before!.order);
    if (later !== null) return later - (after!.order - order);
    return order;
  };
  return new Map(entries.map(entry => {
    const anchors = entry.provenance.sourceSpans.map(span => ({ span, source: sourceMap.get(span.messageId) })).filter(item => item.source).sort((a, b) => a.source!.order - b.source!.order || a.span.start - b.span.start);
    const last = anchors.at(-1);
    const sourceOrder = last?.source?.order ?? null;
    // Use only the entry's asserted occurrence time. A log/import time is never a replacement.
    const timestamp = validTime(entry.provenance.occurredAt) === null ? null : entry.provenance.occurredAt;
    const time = timestamp ? validTime(timestamp)! : sourceFallback(sourceOrder ?? sources.length + 1);
    return [entry.id, { timestamp, sourceOrder, offset: last?.span.start ?? 0, time, band: timestamp ? `time:${time}:source:${sourceOrder ?? 'unknown'}` : `source:${sourceOrder ?? 'unknown'}` }];
  }));
}

/** Condensing strongly connected components gives cycles an explicit, finite layout. */
function componentLayers(ids: string[], edges: GraphLink[], compare: (a: string, b: string) => number): { layers: string[][]; cyclic: Set<string> } {
  const idSet = new Set(ids);
  const following = new Map(ids.map(id => [id, [] as string[]]));
  for (const edge of edges) if (idSet.has(edge.source) && idSet.has(edge.target)) following.get(edge.source)!.push(edge.target);
  let next = 0;
  const index = new Map<string, number>(), low = new Map<string, number>();
  const stack: string[] = [], stacked = new Set<string>(), components: string[][] = [];
  function visit(id: string) {
    index.set(id, next); low.set(id, next++); stack.push(id); stacked.add(id);
    for (const target of following.get(id)!) {
      if (!index.has(target)) { visit(target); low.set(id, Math.min(low.get(id)!, low.get(target)!)); }
      else if (stacked.has(target)) low.set(id, Math.min(low.get(id)!, index.get(target)!));
    }
    if (low.get(id) === index.get(id)) {
      const group: string[] = [];
      let member: string;
      do { member = stack.pop()!; stacked.delete(member); group.push(member); } while (member !== id);
      components.push(group.sort(compare));
    }
  }
  [...ids].sort(compare).forEach(id => { if (!index.has(id)) visit(id); });
  const owner = new Map(components.flatMap((component, i) => component.map(id => [id, i] as const)));
  const ranks = new Map<number, number>(), incoming = new Map(components.map((_, i) => [i, new Set<number>()]));
  for (const edge of edges) {
    const from = owner.get(edge.source), to = owner.get(edge.target);
    if (from !== undefined && to !== undefined && from !== to) incoming.get(to)!.add(from);
  }
  function rank(component: number): number {
    if (ranks.has(component)) return ranks.get(component)!;
    const result = Math.max(0, ...[...incoming.get(component)!].map(parent => rank(parent) + 1));
    ranks.set(component, result); return result;
  }
  const layers: string[][] = [], cyclic = new Set<string>();
  components.forEach((component, i) => {
    (layers[rank(i)] ??= []).push(...component);
    if (component.length > 1 || following.get(component[0])!.includes(component[0])) component.forEach(id => cyclic.add(id));
  });
  layers.forEach(layer => layer.sort(compare));
  return { layers, cyclic };
}

/** The full extraction defines positions once. Filtering and inspection only project this map. */
export function buildGraphModel(bundle: ReasoningBundle, fullView: View): GraphModel {
  const { entries, connections } = fullView;
  if (!entries.length) return { nodes: [], edges: [], warnings: [] };
  const entryMap = new Map(entries.map(entry => [entry.id, entry]));
  const points = moments(bundle, entries);
  const compareEntries = (a: string, b: string) => {
    const left = points.get(a), right = points.get(b);
    return (left?.time ?? 0) - (right?.time ?? 0) || (left?.sourceOrder ?? 0) - (right?.sourceOrder ?? 0) || (left?.offset ?? 0) - (right?.offset ?? 0) || a.localeCompare(b);
  };
  const sorted = [...entries].sort((a, b) => compareEntries(a.id, b.id));
  const bands: string[][] = [], bandByKey = new Map<string, number>(), bandOf = new Map<string, number>();
  sorted.forEach(entry => {
    const key = points.get(entry.id)!.band;
    if (!bandByKey.has(key)) { bandByKey.set(key, bands.length); bands.push([]); }
    const band = bandByKey.get(key)!;
    bands[band].push(entry.id); bandOf.set(entry.id, band);
  });
  const edges = connections.flatMap(relationLinks);
  const roles = new Map(entries.map(entry => [entry.id, new Map<string, string[]>()]));
  connections.forEach(connection => {
    const links = edges.filter(edge => edge.connectionId === connection.id);
    const outputBands = links.filter(edge => edge.source === connection.id).map(edge => bandOf.get(edge.target) ?? 0);
    const inputBands = links.filter(edge => edge.target === connection.id).map(edge => bandOf.get(edge.source) ?? 0);
    // Put a junction before its result, except when it cites a later premise. Those back references stay visible.
    const band = Math.max(0, ...outputBands, ...inputBands);
    bands[band].push(connection.id); bandOf.set(connection.id, band);
    for (const link of links) {
      const id = link.source === connection.id ? link.target : link.source;
      const map = roles.get(id);
      if (map) { const references = map.get(link.role) ?? []; references.push(connection.id); map.set(link.role, references); }
    }
  });
  const predecessor = new Map<string, string[]>();
  for (const edge of edges) predecessor.set(edge.target, [...(predecessor.get(edge.target) ?? []), edge.source]);
  const nodes: GraphNode[] = [], placed = new Map<string, GraphNode>(), warnings: string[] = [];
  const laneCount = Math.min(5, Math.max(2, Math.ceil(Math.sqrt(entries.length))));
  let y = 0;
  for (const band of bands) {
    const compare = (a: string, b: string) => (entryMap.has(a) === entryMap.has(b) ? compareEntries(a, b) : entryMap.has(a) ? 1 : -1);
    const { layers, cyclic } = componentLayers(band, edges, compare);
    if (cyclic.size) warnings.push(`A relation cycle involving ${[...cyclic].join(', ')} shares a time band. Its arrows are preserved without asserting an order within the cycle.`);
    for (const layer of layers) {
      for (let row = 0; row < layer.length; row += laneCount) {
        const items = layer.slice(row, row + laneCount), occupied = new Set<number>();
        const rowHeight = Math.max(...items.map(id => entryMap.has(id) ? ENTRY_SIZE.height : CONNECTION_SIZE.height));
        for (const id of items) {
          const entry = entryMap.get(id), point = points.get(id);
          const previous = (predecessor.get(id) ?? []).map(parent => placed.get(parent)).filter((parent): parent is GraphNode => !!parent);
          const actor = bundle.actors.find(actor => actor.id === entry?.provenance.expressedBy);
          const preferred = previous.length ? previous.reduce((sum, node) => sum + (node.position.x + node.width / 2 - ENTRY_SIZE.width / 2) / LANE_WIDTH, 0) / previous.length : actor?.kind === 'human' ? 0 : actor?.kind === 'agent' ? 1 : Math.floor(laneCount / 2);
          const lane = Array.from({ length: laneCount }, (_, i) => i).filter(i => !occupied.has(i)).sort((a, b) => Math.abs(a - preferred) - Math.abs(b - preferred) || a - b)[0];
          occupied.add(lane);
          const size = entry ? ENTRY_SIZE : CONNECTION_SIZE;
          const node: GraphNode = { id, kind: entry ? 'entry' : 'connection', position: { x: lane * LANE_WIDTH + (ENTRY_SIZE.width - size.width) / 2, y: y + (rowHeight - size.height) / 2 }, ...size, roles: [...(roles.get(id)?.entries() ?? [])].map(([role, connectionIds]) => ({ role, connectionIds })), timestamp: point?.timestamp ?? null, sourceOrder: point?.sourceOrder ?? null, cycle: cyclic.has(id) };
          nodes.push(node); placed.set(id, node);
        }
        y += rowHeight + ROW_GAP;
      }
    }
    y += BAND_GAP;
  }
  for (const edge of edges) edge.pointsToEarlierEntry = (placed.get(edge.target)?.position.y ?? 0) <= (placed.get(edge.source)?.position.y ?? 0);
  return { nodes, edges, warnings };
}

/** Keep exact coordinates and semantic direction when showing a historical or filtered slice. */
export function projectGraphModel(model: GraphModel, view: View): GraphModel {
  const ids = new Set([...view.entries.map(entry => entry.id), ...view.connections.map(connection => connection.id)]);
  const edges = model.edges.filter(edge => ids.has(edge.connectionId) && ids.has(edge.source) && ids.has(edge.target));
  const { cyclic } = componentLayers([...ids], edges, (a, b) => a.localeCompare(b));
  const nodes = model.nodes.filter(node => ids.has(node.id)).map(node => ({ ...node, cycle: cyclic.has(node.id), roles: node.roles.map(role => ({ ...role, connectionIds: role.connectionIds.filter(id => ids.has(id)) })).filter(role => role.connectionIds.length) }));
  return { nodes, edges, warnings: cyclic.size ? ['A visible relation cycle is marked on its cards; no internal chronological order is asserted.'] : [] };
}
