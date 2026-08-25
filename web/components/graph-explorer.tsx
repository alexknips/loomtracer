"use client";

import ELK from "elkjs/lib/elk.bundled.js";
import {
  Background,
  Controls,
  Handle,
  MarkerType,
  MiniMap,
  Position,
  ReactFlow,
  ReactFlowProvider,
  type Edge,
  type Node,
  type NodeProps,
} from "@xyflow/react";
import { useEffect, useMemo, useState } from "react";
import "@xyflow/react/dist/style.css";
import type { GraphNodeRecord, GraphPayload } from "@/types/graph";

type GraphCardData = { record: GraphNodeRecord };
type GraphCardNode = Node<GraphCardData, "graphCard">;

const elk = new ELK();
const kindIcons: Record<string, string> = {
  Decision: "D",
  Evidence: "E",
  Assumption: "A",
  Attempt: "T",
  Option: "O",
  Artifact: "↗",
  Actor: "@",
  Scope: "S",
};

function GraphCard({ data, selected }: NodeProps<GraphCardNode>) {
  const node = data.record;
  return (
    <article className={`graph-card kind-${node.type.toLowerCase()} ${selected ? "selected" : ""}`}>
      <Handle type="target" position={Position.Left} className="node-handle" />
      <div className="graph-card-head">
        <span className="kind-icon" aria-hidden="true">{kindIcons[node.type] ?? "•"}</span>
        <span>{node.type}</span>
        <span className={`status status-${node.status}`}>{node.status}</span>
      </div>
      <h3>{node.title}</h3>
      <p>{node.summary || "No summary provided."}</p>
      <footer>
        <span>{node.actor?.display_name ?? "Unknown actor"}</span>
        <time>{new Date(node.updated_at).toLocaleDateString(undefined, { month: "short", day: "numeric" })}</time>
      </footer>
      <Handle type="source" position={Position.Right} className="node-handle" />
    </article>
  );
}

const nodeTypes = { graphCard: GraphCard };

async function layoutGraph(nodes: GraphCardNode[], edges: Edge[]): Promise<GraphCardNode[]> {
  const graph = await elk.layout({
    id: "root",
    layoutOptions: {
      "elk.algorithm": "layered",
      "elk.direction": "RIGHT",
      "elk.spacing.nodeNode": "52",
      "elk.layered.spacing.nodeNodeBetweenLayers": "120",
      "elk.layered.nodePlacement.strategy": "NETWORK_SIMPLEX",
    },
    children: nodes.map((node) => ({ id: node.id, width: 300, height: 188 })),
    edges: edges.map((edge) => ({ id: edge.id, sources: [edge.source], targets: [edge.target] })),
  });
  const positions = new Map(graph.children?.map((node) => [node.id, node]) ?? []);
  return nodes.map((node) => ({
    ...node,
    position: { x: positions.get(node.id)?.x ?? 0, y: positions.get(node.id)?.y ?? 0 },
  }));
}

function Explorer({ payload }: { payload: GraphPayload }) {
  const [query, setQuery] = useState("");
  const [typeFilter, setTypeFilter] = useState("all");
  const [statusFilter, setStatusFilter] = useState("all");
  const [actorFilter, setActorFilter] = useState("all");
  const [scopeFilter, setScopeFilter] = useState("");
  const [dateFilter, setDateFilter] = useState("all");
  const [dateCutoff, setDateCutoff] = useState<number | null>(null);
  const [searchResults, setSearchResults] = useState<Array<{ id: string; type: string; status: string; title: string }>>([]);
  const [selectedId, setSelectedId] = useState(payload.root_id);
  const [nodes, setNodes] = useState<GraphCardNode[]>([]);

  const filteredRecords = useMemo(() => payload.nodes.filter((node) => {
    const corpus = `${node.title} ${node.summary} ${node.scope_refs.join(" ")}`.toLowerCase();
    return (!query || corpus.includes(query.toLowerCase()))
      && (typeFilter === "all" || node.type === typeFilter)
      && (statusFilter === "all" || node.status === statusFilter)
      && (actorFilter === "all" || node.created_by_actor_id === actorFilter)
      && (!scopeFilter || `${node.scope_type ?? ""} ${node.scope_refs.join(" ")}`.toLowerCase().includes(scopeFilter.toLowerCase()))
      && (dateCutoff === null || new Date(node.updated_at).getTime() >= dateCutoff);
  }), [actorFilter, dateCutoff, payload.nodes, query, scopeFilter, statusFilter, typeFilter]);

  useEffect(() => {
    if (query.trim().length < 2) return;
    const controller = new AbortController();
    const timer = window.setTimeout(() => {
      void fetch(`/api/search?q=${encodeURIComponent(query)}`, { cache: "no-store", signal: controller.signal })
        .then((response) => response.ok ? response.json() : { results: [] })
        .then((body) => setSearchResults(body.results ?? []))
        .catch(() => undefined);
    }, 250);
    return () => { window.clearTimeout(timer); controller.abort(); };
  }, [query]);

  const visibleIds = useMemo(() => new Set(filteredRecords.map((node) => node.id)), [filteredRecords]);
  const edges = useMemo<Edge[]>(() => payload.edges
    .filter((edge) => visibleIds.has(edge.source_id) && visibleIds.has(edge.target_id))
    .map((edge) => ({
      id: edge.id,
      source: edge.source_id,
      target: edge.target_id,
      label: edge.edge_type.replaceAll("_", " "),
      type: "smoothstep",
      markerEnd: { type: MarkerType.ArrowClosed, width: 16, height: 16 },
      animated: edge.metadata.proposed === true,
      className: edge.metadata.proposed === true || edge.metadata.inferred === true ? "edge-proposed" : undefined,
    })), [payload.edges, visibleIds]);

  useEffect(() => {
    const nextNodes: GraphCardNode[] = filteredRecords.map((record) => ({
      id: record.id,
      type: "graphCard",
      data: { record },
      position: { x: 0, y: 0 },
    }));
    void layoutGraph(nextNodes, edges).then(setNodes);
  }, [edges, filteredRecords]);

  const selected = payload.nodes.find((node) => node.id === selectedId) ?? null;
  const connectedEdges = selected
    ? payload.edges.filter((edge) => edge.source_id === selected.id || edge.target_id === selected.id)
    : [];

  return (
    <div className="explorer">
      <aside className="filter-panel">
        <div>
          <span className="eyebrow">Graph controls</span>
          <h1>Decision flow</h1>
          <p>Trace what was decided, what supported it, and what changed.</p>
        </div>
        <label>
          <span>Search graph</span>
          <input value={query} onChange={(event) => { const value = event.target.value; setQuery(value); if (value.trim().length < 2) setSearchResults([]); }} placeholder="Title, scope, evidence…" />
          {searchResults.length > 0 && <div className="search-results" role="listbox">{searchResults.map((result) => <a key={result.id} href={`/graph?root=${encodeURIComponent(result.id)}`}><b>{result.title}</b><small>{result.type} · {result.status}</small></a>)}</div>}
        </label>
        <label>
          <span>Node type</span>
          <select value={typeFilter} onChange={(event) => setTypeFilter(event.target.value)}>
            <option value="all">All types</option>
            {[...new Set(payload.nodes.map((node) => node.type))].map((type) => <option key={type}>{type}</option>)}
          </select>
        </label>
        <label>
          <span>Actor</span>
          <select value={actorFilter} onChange={(event) => setActorFilter(event.target.value)}>
            <option value="all">All actors</option>
            {[...new Map(payload.nodes.filter((node) => node.created_by_actor_id && node.actor).map((node) => [node.created_by_actor_id!, node.actor!])).entries()].map(([id, actor]) => <option value={id} key={id}>{actor.display_name}</option>)}
          </select>
        </label>
        <label>
          <span>Scope</span>
          <input value={scopeFilter} onChange={(event) => setScopeFilter(event.target.value)} placeholder="repo, feature, file…" />
        </label>
        <label>
          <span>Updated</span>
          <select value={dateFilter} onChange={(event) => { const value = event.target.value; setDateFilter(value); setDateCutoff(value === "all" ? null : Date.now() - Number(value) * 86_400_000); }}><option value="all">Any time</option><option value="7">Last 7 days</option><option value="30">Last 30 days</option><option value="365">Last year</option></select>
        </label>
        <label>
          <span>Status</span>
          <select value={statusFilter} onChange={(event) => setStatusFilter(event.target.value)}>
            <option value="all">All statuses</option>
            {[...new Set(payload.nodes.map((node) => node.status))].map((status) => <option key={status}>{status}</option>)}
          </select>
        </label>
        <div className="scope-summary">
          <span><b>{filteredRecords.length}</b> nodes</span>
          <span><b>{edges.length}</b> relations</span>
          <span><b>2</b> hops</span>
        </div>
        <div className="legend">
          <span><i className="legend-decision" /> Decision</span>
          <span><i className="legend-evidence" /> Evidence</span>
          <span><i className="legend-assumption" /> Assumption</span>
        </div>
      </aside>
      <main className="graph-canvas" aria-label="Decision graph">
        <ReactFlow
          nodes={nodes}
          edges={edges}
          nodeTypes={nodeTypes}
          onNodeClick={(_, node) => setSelectedId(node.id)}
          nodesDraggable={false}
          nodesConnectable={false}
          minZoom={0.35}
          maxZoom={1.4}
          fitView
          fitViewOptions={{ padding: 0.22 }}
        >
          <Background color="#d6d6cf" gap={28} size={1} />
          <MiniMap pannable zoomable nodeColor="#343b35" maskColor="rgba(248,248,244,.78)" />
          <Controls showInteractive={false} />
        </ReactFlow>
        <div className="canvas-caption">Showing the focused decision and two degrees of context</div>
      </main>
      <aside className={`detail-panel ${selected ? "open" : ""}`} aria-live="polite">
        {selected ? (
          <>
            <div className="detail-kicker"><span>{selected.type}</span><span className={`status status-${selected.status}`}>{selected.status}</span></div>
            <h2>{selected.title}</h2>
            <p className="detail-summary">{selected.summary}</p>
            {selected.body && <p className="detail-body">{selected.body}</p>}
            <dl>
              <div><dt>ID</dt><dd>{selected.id}</dd></div>
              <div><dt>Created by</dt><dd>{selected.actor?.display_name ?? "Unknown"}</dd></div>
              <div><dt>Scope</dt><dd>{selected.scope_type ?? "Unspecified"}</dd></div>
              <div><dt>Confidence</dt><dd>{selected.confidence ?? "Unspecified"}</dd></div>
              <div><dt>Reversibility</dt><dd>{selected.reversibility ?? "Unspecified"}</dd></div>
              <div><dt>Consequence</dt><dd>{selected.consequence ?? "Unspecified"}</dd></div>
              <div><dt>Created</dt><dd>{new Date(selected.created_at).toLocaleString()}</dd></div>
              <div><dt>Updated</dt><dd>{new Date(selected.updated_at).toLocaleString()}</dd></div>
              {selected.ratified_at && <div><dt>Ratified</dt><dd>{new Date(selected.ratified_at).toLocaleString()}</dd></div>}
            </dl>
            {selected.scope_refs.length > 0 && <div className="tag-row">{selected.scope_refs.map((ref) => <span key={ref}>{ref}</span>)}</div>}
            {selected.reconsider_conditions.length > 0 && (
              <section className="detail-section">
                <h3>Reconsider if</h3>
                <ul>{selected.reconsider_conditions.map((item) => <li key={item}>{item}</li>)}</ul>
              </section>
            )}
            {(typeof selected.metadata.evidence_summary === "string" || typeof selected.metadata.source_ref === "string") && <section className="detail-section"><h3>Evidence & source</h3>{typeof selected.metadata.evidence_summary === "string" && <p>{selected.metadata.evidence_summary}</p>}{typeof selected.metadata.source_ref === "string" && <p><b>Source:</b> {selected.metadata.source_ref}</p>}</section>}
            <section className="detail-section"><h3>Record metadata</h3><pre className="metadata-block">{JSON.stringify(selected.metadata, null, 2)}</pre></section>
            <section className="detail-section">
              <h3>Relations</h3>
              {connectedEdges.map((edge) => <p key={edge.id}><b>{edge.edge_type.replaceAll("_", " ")}</b> · {edge.source_id === selected.id ? edge.target_id : edge.source_id}</p>)}
            </section>
            <section className="detail-section audit-list">
              <h3>History</h3>
              {(payload.audits[selected.id] ?? []).map((audit) => (
                <p key={audit.id}><span>{audit.action}</span><time>{new Date(audit.created_at).toLocaleString()}</time></p>
              ))}
              {(payload.audits[selected.id] ?? []).length === 0 && <p>No lifecycle events recorded.</p>}
            </section>
          </>
        ) : <p>Select a node to inspect its record and history.</p>}
      </aside>
    </div>
  );
}

export function GraphExplorer({ payload }: { payload: GraphPayload }) {
  return <ReactFlowProvider><Explorer payload={payload} /></ReactFlowProvider>;
}
