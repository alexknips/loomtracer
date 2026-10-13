import { useCallback, useEffect, useMemo, useRef } from 'react';
import { ReactFlow, Background, Controls, MiniMap, Handle, MarkerType, Position, BaseEdge, getBezierPath, type Node, type Edge, type NodeProps, type NodeTypes, type EdgeProps, type EdgeTypes, type ReactFlowInstance } from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import type { ReasoningBundle, View, Relation } from '@reasoning';
import { entryText } from '@reasoning';
import { buildGraphModel, projectGraphModel, type GraphRole } from '@reasoning';

// Agreed Loomtracer relationship colors, using the supplied brand palette.
const relationColors: Partial<Record<Relation, string>> = {
  answers: '#332B32', responds_to: '#332B32',
  supports: '#1D90CD', adopts: '#1D90CD',
  challenges: '#E25432', withdraws: '#E25432',
  compares: '#F3BA1E', revises: '#F3BA1E',
};
const relationLegend = [
  { color: relationColors.answers, label: 'Answers / responds to' },
  { color: relationColors.supports, label: 'Supports / adopts' },
  { color: relationColors.challenges, label: 'Challenges / withdraws' },
  { color: relationColors.compares, label: 'Compares / revises' },
];

function CurvedConnection(props: EdgeProps) {
  const [path] = getBezierPath({ ...props, curvature: .3 });
  const strokeWidth = Number(props.style?.strokeWidth ?? 2);
  return <>
    {/* Yellow needs an Ink outline to remain visible against Ivory. */}
    {props.style?.stroke === '#F3BA1E' && <path d={path} fill="none" stroke="#332B32" strokeWidth={strokeWidth + 1.5} strokeDasharray={props.style.strokeDasharray} strokeLinecap="round" pointerEvents="none" aria-hidden="true" />}
    <BaseEdge id={props.id} path={path} style={props.style} markerEnd={props.markerEnd} markerStart={props.markerStart} interactionWidth={20} />
  </>;
}
const edgeTypes: EdgeTypes = { reasoningBezier: CurvedConnection };

export type Selection = { kind: 'entry' | 'connection' | 'source'; id: string };
type TraceData = {
  label: string; id: string; kind: 'entry' | 'connection'; byline: string;
  inferred: boolean; selected: boolean; related: boolean; roles: GraphRole[];
  labels: string[]; connectionColor?: string;
  timeLabel: string; timestamp: string | null; cycle: boolean;
};
type TraceNode = Node<TraceData>;
function TraceCard({ data }: NodeProps<TraceNode>) {
  return <div style={data.connectionColor ? { borderColor: data.connectionColor } : undefined} role="button" tabIndex={0} aria-label={`${data.kind} ${data.id}`} aria-pressed={data.selected} data-testid={`graph-node-${data.id}`} onKeyDown={event => { if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); event.currentTarget.click(); } }} className={`trace-node ${data.kind === 'connection' ? 'trace-junction' : ''} ${data.selected ? 'is-selected' : ''} ${data.related ? 'is-related' : ''}`}>
    <Handle type="target" position={Position.Top} />
    <div className="trace-node-meta">{data.kind === 'connection' ? 'CONNECTION' : data.byline}{data.inferred && <span>INFERRED</span>}{data.cycle && <span>RELATION CYCLE</span>}</div>
    {data.kind === 'entry' && <div className="trace-node-roles" aria-label="Roles in this map">{data.roles.length ? data.roles.slice(0, 3).map(({ role, connectionIds }) => <span className="trace-role" key={role} title={`Role in ${connectionIds.join(', ')}`}>{role.replaceAll('_', ' ')}</span>) : <><span className="trace-role" title="Retrieval label; no relationship is asserted">{data.labels.includes('question') ? 'Question' : data.labels[0] ?? 'Contribution'}</span><span className="trace-role">No links yet</span></>}{data.roles.length > 3 && <span className="trace-role" title={data.roles.slice(3).map(item => item.role).join(', ')}>+{data.roles.length - 3}</span>}</div>}
    <div className="trace-node-label">{data.connectionColor && <span className="connection-color-dot" style={{ backgroundColor: data.connectionColor }} aria-hidden="true" />}{data.label}</div>
    {data.kind === 'entry' && <time className="trace-node-time" dateTime={data.timestamp ?? undefined} title={data.timestamp ?? 'No original timestamp is recorded; position follows source order.'}>{data.timeLabel}</time>}
    <span className="trace-node-id">{data.id}</span>
    <Handle type="source" position={Position.Bottom} />
  </div>;
}
const nodeTypes: NodeTypes = { trace: TraceCard };
const formatTime = (timestamp: string | null, sourceOrder: number | null) => timestamp
  ? `${new Date(timestamp).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })}${sourceOrder === null ? '' : ` · #${sourceOrder}`}`
  : `Time unknown${sourceOrder === null ? '' : ` · source ${sourceOrder}`}`;

export default function GraphView({ bundle, view, fullView = view, selected, onSelect }: { bundle: ReasoningBundle; view: View; fullView?: View; selected: Selection | null; onSelect: (selection: Selection) => void }) {
  const flow = useRef<ReactFlowInstance<TraceNode, Edge> | null>(null);
  const fittedDataset = useRef<string | null>(null);
  const model = useMemo(() => buildGraphModel(bundle, fullView), [bundle, fullView]);
  const visible = useMemo(() => projectGraphModel(model, view), [model, view]);
  const entryMap = useMemo(() => new Map(fullView.entries.map(entry => [entry.id, entry])), [fullView.entries]);
  const connectionMap = useMemo(() => new Map(fullView.connections.map(connection => [connection.id, connection])), [fullView.connections]);
  const selectedConnections = new Set(selected?.kind === 'connection' ? [selected.id] : selected?.kind === 'entry' ? visible.edges.filter(edge => edge.source === selected.id || edge.target === selected.id).map(edge => edge.connectionId) : []);
  const relatedIds = new Set(visible.edges.filter(edge => selectedConnections.has(edge.connectionId)).flatMap(edge => [edge.source, edge.target]));
  const nodes: TraceNode[] = visible.nodes.map(node => {
    const entry = entryMap.get(node.id), connection = connectionMap.get(node.id);
    const isSelected = selected?.kind === node.kind && selected.id === node.id;
    const label = entry ? (entry.summary ?? entryText(bundle, entry)).slice(0, 165) : connection!.relation.replaceAll('_', ' ');
    return {
      id: node.id, type: 'trace', position: node.position, width: node.width, height: node.height, selected: isSelected,
      data: { id: node.id, kind: node.kind, label, connectionColor: connection ? (relationColors[connection.relation] ?? '#332B32') : undefined, labels: entry?.labels ?? [], byline: entry ? `${bundle.actors.find(actor => actor.id === entry.provenance.expressedBy)?.name ?? 'Unstated author'}${entry.summary ? ' · summary' : ''}` : '', inferred: (entry ?? connection)!.provenance.basis === 'inferred', selected: isSelected, related: relatedIds.has(node.id), roles: node.roles, timestamp: node.timestamp, timeLabel: formatTime(node.timestamp, node.sourceOrder), cycle: node.cycle },
      ariaLabel: `${node.kind === 'entry' ? 'Entry' : 'Connection'} ${node.id}: ${label.slice(0, 90)}`,
    };
  });
  const edges: Edge[] = visible.edges.map(edge => {
    const highlighted = selectedConnections.has(edge.connectionId);
    const color = (relationColors[connectionMap.get(edge.connectionId)!.relation] ?? '#332B32');
    return { ...edge, type: 'reasoningBezier', animated: false, selected: highlighted, ariaLabel: `${edge.role.replaceAll('_', ' ')}: ${edge.source} → ${edge.target}`, style: { stroke: color, strokeWidth: highlighted ? 3.8 : 2.2, strokeLinecap: 'round', ...(edge.pointsToEarlierEntry ? { strokeDasharray: '6 4' } : {}) }, markerEnd: { type: MarkerType.ArrowClosed, color, width: 16, height: 16, markerUnits: 'userSpaceOnUse' }, data: { connectionId: edge.connectionId } };
  });
  const datasetKey = `${bundle.id}/${fullView.run.id}/${model.nodes.map(node => node.id).join('|')}`;
  const initialNodes = useRef(nodes);
  const initialSelection = useRef(selected);
  useEffect(() => { initialNodes.current = nodes; initialSelection.current = selected; }, [nodes, selected]);
  const fitInitial = useCallback(() => {
    if (!flow.current || fittedDataset.current === datasetKey || !initialNodes.current.length) return;
    fittedDataset.current = datasetKey;
    // Start at readable text size. Subsequent inspection never changes the viewport.
    const first = [...initialNodes.current].sort((a, b) => a.position.y - b.position.y);
    const anchor = first.find(node => node.id === initialSelection.current?.id) ?? first.find(node => node.data.kind === 'entry') ?? first[0];
    requestAnimationFrame(() => { void flow.current?.setViewport({ x: 32 - anchor.position.x * .85, y: 90 - anchor.position.y * .85, zoom: .85 }, { duration: 0 }); });
  }, [datasetKey]);
  useEffect(() => { fitInitial(); }, [fitInitial, nodes.length]);
  const selectedVisible = selected && selected.kind !== 'source' && nodes.some(node => node.id === selected.id);
  const backReferences = new Set(visible.edges.filter(edge => edge.pointsToEarlierEntry).map(edge => edge.connectionId)).size;
  return <><div className="graph-color-legend" aria-label="Connection color legend">{relationLegend.map(item => <span key={item.label}><span className="connection-color-dot" style={{ backgroundColor: item.color }} aria-hidden="true" />{item.label}</span>)}</div><div className="graph-surface" data-testid="graph-canvas">
    <div className="graph-toolbar"><span>Time ↓ · {view.entries.length} contributions</span><div className="graph-toolbar-actions"><button className="text-button" disabled={!selectedVisible} onClick={() => { if (selectedVisible) void flow.current?.fitView({ nodes: [{ id: selected.id }], minZoom: .75, maxZoom: 1, padding: .9, duration: 0 }); }}>Find selected</button><button className="text-button" onClick={() => { void flow.current?.fitView({ padding: .15, minZoom: .015, maxZoom: .85, duration: 0 }); }}>Fit map</button></div></div>
    <ReactFlow onInit={instance => { flow.current = instance; fitInitial(); }} nodes={nodes} edges={edges} nodeTypes={nodeTypes} edgeTypes={edgeTypes} minZoom={.015} maxZoom={1.6} panOnScroll zoomOnScroll={false} zoomOnPinch zoomOnDoubleClick={false} autoPanOnNodeFocus={false} nodesDraggable={false} nodesConnectable={false} onNodeClick={(_, node) => onSelect({ kind: node.data.kind, id: node.data.id })} onEdgeClick={(_, edge) => onSelect({ kind: 'connection', id: String(edge.data?.connectionId) })} proOptions={{ hideAttribution: false }}>
      <Background gap={22} size={1} color="#332B3218" /><Controls showInteractive={false} fitViewOptions={{ padding: .15, minZoom: .015, maxZoom: .85, duration: 0 }} />
      <MiniMap position="bottom-right" pannable zoomable ariaLabel="Reasoning graph minimap. Drag to pan; scroll to zoom." nodeColor={node => node.data.connectionColor ? String(node.data.connectionColor) : node.selected ? '#1D90CD' : '#FDF8F4'} nodeStrokeColor={node => node.selected ? '#1D90CD' : '#332B32'} nodeStrokeWidth={3} maskColor="#332B3220" bgColor="#FDF8F4" />
    </ReactFlow>
    <div className="graph-caption">Entries follow occurrence time; undated entries use source order. Spacing is compressed. Roles appear on entries; arrows show the relationship direction.{backReferences > 0 && ` ${backReferences} relation${backReferences === 1 ? '' : 's'} link back to an earlier entry (dashed).`}{visible.warnings.length > 0 && <span title={visible.warnings.join('\n')}> Relation cycles are marked on cards.</span>}</div>
  </div></>;
}
