"use client";

import ELK from "elkjs/lib/elk.bundled.js";
import {
  Background,
  BaseEdge,
  Controls,
  EdgeLabelRenderer,
  Handle,
  MarkerType,
  MiniMap,
  Position,
  ReactFlow,
  ReactFlowProvider,
  getBezierPath,
  type Edge,
  type EdgeProps,
  type Node,
  type NodeProps,
} from "@xyflow/react";
import { useEffect, useMemo, useState } from "react";
import "@xyflow/react/dist/style.css";
import { ProjectSelector } from "@/components/project-selector";
import { NodeOrganization } from "@/components/node-organization";
import { projectPath } from "@/lib/projects";
import type { GraphNodeRecord, GraphPayload } from "@/types/graph";

type GraphCardData = { record: GraphNodeRecord; projectLabel: string };
type GraphCardNode = Node<GraphCardData, "graphCard">;
type RelationEdgeData = { label: string; color: string; labelOffset: number; dimmed: boolean };
type RelationEdge = Edge<RelationEdgeData, "relation">;

const elk = new ELK();
const kindIcons: Record<string, string> = {
  Decision: "D",
  Evidence: "E",
  Assumption: "A",
  Attempt: "T",
  Option: "O",
  Artifact: "↗",
  Scope: "S",
};

const relationStyles: Record<string, { color: string; labelOffset: number }> = {
  supports: { color: "#2f6f5e", labelOffset: 0 },
  contradicts: { color: "#b04e45", labelOffset: 16 },
  depends_on: { color: "#4f70a8", labelOffset: 0 },
  supersedes: { color: "#725e9c", labelOffset: -16 },
  invalidates: { color: "#8f3d49", labelOffset: 16 },
  rejects: { color: "#a05a3d", labelOffset: 12 },
  chosen_over: { color: "#2f7f82", labelOffset: -12 },
  based_on_assumption: { color: "#9a6a1b", labelOffset: 0 },
  applies_to_scope: { color: "#68736f", labelOffset: 0 },
};
const fallbackRelationStyle = { color: "#68736f", labelOffset: 0 };

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
      <div className="node-project">{node.inherited_from_project_id ? "Inherited · " : ""}{data.projectLabel}</div>
      <h3>{node.title}</h3>
      <p>{node.summary || "No summary provided."}</p>
      <footer>
        <span>{node.actor?.display_name ?? "Unknown author"}</span>
        <time>{new Date(node.updated_at).toLocaleDateString(undefined, { month: "short", day: "numeric" })}</time>
      </footer>
      <Handle type="source" position={Position.Right} className="node-handle" />
    </article>
  );
}

const nodeTypes = { graphCard: GraphCard };

function RelationConnector({
  id,
  sourceX,
  sourceY,
  targetX,
  targetY,
  sourcePosition,
  targetPosition,
  markerEnd,
  style,
  data,
}: EdgeProps<RelationEdge>) {
  const [path, labelX, labelY] = getBezierPath({
    sourceX,
    sourceY,
    targetX,
    targetY,
    sourcePosition,
    targetPosition,
    curvature: 0.32,
  });
  const color = data?.color ?? fallbackRelationStyle.color;
  return (
    <>
      <BaseEdge id={id} path={path} markerEnd={markerEnd} style={{ ...style, stroke: color }} />
      <EdgeLabelRenderer>
        <span
          className={`relation-label nodrag nopan ${data?.dimmed ? "is-dimmed" : ""}`}
          style={{
            color,
            borderColor: `${color}55`,
            transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY + (data?.labelOffset ?? 0)}px)`,
          }}
        >
          {data?.label ?? "relation"}
        </span>
      </EdgeLabelRenderer>
    </>
  );
}

const edgeTypes = { relation: RelationConnector };

async function layoutGraph(nodes: GraphCardNode[], edges: Edge[]): Promise<GraphCardNode[]> {
  const graph = await elk.layout({
    id: "root",
    layoutOptions: {
      "elk.algorithm": "layered",
      "elk.direction": "RIGHT",
      "elk.spacing.nodeNode": "52",
      "elk.spacing.componentComponent": "52",
      "elk.layered.spacing.nodeNodeBetweenLayers": "120",
      "elk.layered.nodePlacement.strategy": "NETWORK_SIMPLEX",
    },
    children: nodes.map((node) => ({ id: node.id, width: 300, height: 264 })),
    edges: edges.map((edge) => ({ id: edge.id, sources: [edge.source], targets: [edge.target] })),
  });
  const positions = new Map(graph.children?.map((node) => [node.id, node]) ?? []);
  return nodes.map((node) => ({
    ...node,
    position: { x: positions.get(node.id)?.x ?? 0, y: positions.get(node.id)?.y ?? 0 },
  }));
}

function Explorer({ payload, initialFocusId }: { payload: GraphPayload; initialFocusId: string | null }) {
  const [query, setQuery] = useState("");
  const [typeFilter, setTypeFilter] = useState("all");
  const [statusFilter, setStatusFilter] = useState("all");
  const [actorFilter, setActorFilter] = useState("all");
  const [scopeFilter, setScopeFilter] = useState("");
  const [dateFilter, setDateFilter] = useState("all");
  const [dateCutoff, setDateCutoff] = useState<number | null>(null);
  const [searchResults, setSearchResults] = useState<Array<{ id: string; type: string; status: string; title: string; project_path?: string; inherited?: boolean }>>([]);
  const [selectedId, setSelectedId] = useState(
    initialFocusId ?? (payload.view === "neighborhood" ? payload.root_id : null),
  );
  const [nodes, setNodes] = useState<GraphCardNode[]>([]);

  const projects = payload.projects ?? [];
  const projectFilter = payload.project_filter ?? "all";
  function graphHref(root?: string | null, view = "all", depth?: number, inherited = payload.include_inherited) {
    const params = new URLSearchParams({ project: projectFilter, view });
    if (root) params.set("root", root);
    if (depth !== undefined) params.set("depth", String(depth));
    if (inherited) params.set("inherited", "true");
    return `/graph?${params}`;
  }

  const filteredRecords = useMemo(() => payload.nodes.filter((node) => {
    const corpus = `${node.title} ${node.summary} ${node.scope_refs.join(" ")} ${node.actor?.display_name ?? ""}`.toLowerCase();
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
      void fetch(`/api/search?q=${encodeURIComponent(query)}&project=${encodeURIComponent(projectFilter)}`, { cache: "no-store", signal: controller.signal })
        .then((response) => response.ok ? response.json() : { results: [] })
        .then((body) => setSearchResults(body.results ?? []))
        .catch(() => undefined);
    }, 250);
    return () => { window.clearTimeout(timer); controller.abort(); };
  }, [query, projectFilter]);

  const visibleIds = useMemo(() => new Set(filteredRecords.map((node) => node.id)), [filteredRecords]);
  const highlightedIds = useMemo(() => {
    if (!selectedId) return null;
    const ids = new Set([selectedId]);
    for (const edge of payload.edges) {
      if (edge.source_id === selectedId) ids.add(edge.target_id);
      if (edge.target_id === selectedId) ids.add(edge.source_id);
    }
    return ids;
  }, [payload.edges, selectedId]);
  const edges = useMemo<RelationEdge[]>(() => payload.edges
    .filter((edge) => visibleIds.has(edge.source_id) && visibleIds.has(edge.target_id))
    .map((edge) => {
      const relationStyle = relationStyles[edge.edge_type] ?? fallbackRelationStyle;
      const dimmed = selectedId !== null && edge.source_id !== selectedId && edge.target_id !== selectedId;
      const classNames = [
        edge.metadata.proposed === true || edge.metadata.inferred === true ? "edge-proposed" : "",
        dimmed ? "edge-dimmed" : "",
      ].filter(Boolean).join(" ");
      return {
        id: edge.id,
        source: edge.source_id,
        target: edge.target_id,
        type: "relation",
        data: {
          label: edge.edge_type.replaceAll("_", " "),
          color: relationStyle.color,
          labelOffset: relationStyle.labelOffset,
          dimmed,
        },
        style: { stroke: relationStyle.color },
        markerEnd: { type: MarkerType.ArrowClosed, width: 16, height: 16, color: relationStyle.color },
        animated: edge.metadata.proposed === true,
        className: classNames || undefined,
      };
    }), [payload.edges, selectedId, visibleIds]);
  const visibleRelationTypes = useMemo(
    () => [...new Set(edges.map((edge) => edge.data?.label ?? "relation"))],
    [edges],
  );

  useEffect(() => {
    const nextNodes: GraphCardNode[] = filteredRecords.map((record) => ({
      id: record.id,
      type: "graphCard",
      data: { record, projectLabel: projectPath(payload.projects ?? [], record.project_id) },
      position: { x: 0, y: 0 },
      className: highlightedIds && !highlightedIds.has(record.id) ? "node-dimmed" : undefined,
    }));
    let active = true;
    void layoutGraph(nextNodes, edges).then(result => { if (active) setNodes(result); });
    return () => { active = false; };
  }, [edges, filteredRecords, highlightedIds, payload.projects]);

  const selected = [...payload.nodes, ...(payload.inherited_nodes ?? [])].find((node) => node.id === selectedId) ?? null;
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
        <ProjectSelector projects={projects} value={projectFilter} />
        <nav className="view-modes" aria-label="Graph view">
          <a className={payload.view === "all" ? "active" : ""} href={graphHref(selectedId)}>All</a>
          <a className={payload.view === "neighborhood" && payload.depth === 1 ? "active" : ""} href={graphHref(selectedId ?? payload.root_id, "neighborhood", 1)}>1 hop</a>
          <a className={payload.view === "neighborhood" && payload.depth === 2 ? "active" : ""} href={graphHref(selectedId ?? payload.root_id, "neighborhood", 2)}>2 hops</a>
        </nav>
        {selectedId && <button type="button" className="clear-focus" onClick={() => setSelectedId(null)}>Clear emphasized focus</button>}
        <label>
          <span>Search graph</span>
          <input value={query} onChange={(event) => { const value = event.target.value; setQuery(value); if (value.trim().length < 2) setSearchResults([]); }} placeholder="Title, author, scope, evidence…" />
          {searchResults.length > 0 && <div className="search-results" role="listbox">{searchResults.map((result) => <a key={result.id} href={graphHref(result.id, "all", undefined, result.inherited || payload.include_inherited)}><b>{result.title}</b><small>{result.type} · {result.status} · {result.inherited ? "Inherited from " : ""}{result.project_path}</small></a>)}</div>}
        </label>
        <label>
          <span>Node type</span>
          <select value={typeFilter} onChange={(event) => setTypeFilter(event.target.value)}>
            <option value="all">All types</option>
            {[...new Set(payload.nodes.map((node) => node.type))].map((type) => <option key={type}>{type}</option>)}
          </select>
        </label>
        <label>
          <span>Author</span>
          <select value={actorFilter} onChange={(event) => setActorFilter(event.target.value)}>
            <option value="all">All authors</option>
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
        {(payload.inherited_nodes?.length ?? 0) > 0 && <section className="inherited-guidance">
          <h2>Inherited guidance</h2>
          <p>Active parent decisions. Workflow and task restrictions still apply.</p>
          <a href={graphHref(null, "all", undefined, !payload.include_inherited)}>{payload.include_inherited ? "Hide inherited nodes on canvas" : "Show inherited nodes on canvas"}</a>
          {payload.inherited_nodes?.map(node => <button type="button" key={node.id} onClick={() => setSelectedId(node.id)}><b>{node.title}</b><small>{projectPath(projects, node.project_id)}</small></button>)}
        </section>}
        {payload.warnings?.map(warning => <p className="graph-warning" key={warning}>{warning}</p>)}
        {payload.truncated && <p className="graph-warning">Canvas limit reached. Search still covers the full selected project.</p>}
        <div className="scope-summary">
          <span><b>{filteredRecords.length}</b> of {payload.total_nodes ?? payload.nodes.length} nodes</span>
          <span><b>{edges.length}</b> of {payload.total_edges ?? payload.edges.length} relations</span>
          <span><b>{payload.view === "all" ? "All" : payload.depth ?? 1}</b> {payload.view === "all" ? "canvas" : (payload.depth ?? 1) === 1 ? "hop" : "hops"}</span>
        </div>
        <div className="legend">
          <b>Nodes</b>
          <span><i className="legend-decision" /> Decision</span>
          <span><i className="legend-evidence" /> Evidence</span>
          <span><i className="legend-assumption" /> Assumption</span>
          <b className="relation-legend-title">Relations</b>
          {visibleRelationTypes.map((relation) => {
            const relationStyle = relationStyles[relation.replaceAll(" ", "_")] ?? fallbackRelationStyle;
            return <span key={relation}><i className="relation-swatch" style={{ background: relationStyle.color }} /> {relation}</span>;
          })}
        </div>
      </aside>
      <main className="graph-canvas" aria-label="Decision graph">
        <ReactFlow
          nodes={nodes}
          edges={edges}
          nodeTypes={nodeTypes}
          edgeTypes={edgeTypes}
          onNodeClick={(_, node) => setSelectedId(node.id)}
          onPaneClick={() => setSelectedId(null)}
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
        {nodes.length === 0 && <div className="graph-empty"><h2>No entries in this view</h2><p>Choose another project or adjust the filters.</p></div>}
        <div className="canvas-caption">{selectedId ? "Direct relationships emphasized · click empty canvas to clear" : payload.view === "all" ? "Project view · select a node to emphasize its direct relationships" : `Showing ${payload.depth ?? 1} ${(payload.depth ?? 1) === 1 ? "degree" : "degrees"} of context`}</div>
      </main>
      <aside className={`detail-panel ${selected ? "open" : ""}`} aria-live="polite">
        {selected ? (
          <>
            <div className="detail-kicker"><span>{selected.type}</span><span className={`status status-${selected.status}`}>{selected.status}</span></div>
            <h2>{selected.title}</h2>
            <p className="detail-summary">{selected.summary}</p>
            {selected.body && <p className="detail-body">{selected.body}</p>}
            <dl>
              <div><dt>Project</dt><dd>{projectPath(projects, selected.project_id)}{selected.inherited_from_project_id ? " · inherited" : ""}</dd></div>
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
            <div className="detail-actions">
              <button type="button" className="button secondary" onClick={() => setSelectedId(null)}>Clear focus</button>
              <a className="button secondary" href={graphHref(selected.id, "neighborhood", 1, Boolean(selected.inherited_from_project_id) || payload.include_inherited)}>Isolate 1 hop</a>
              <a className="button secondary" href={graphHref(selected.id, "neighborhood", 2, Boolean(selected.inherited_from_project_id) || payload.include_inherited)}>Isolate 2 hops</a>
            </div>
            {selected.inherited_from_project_id
              ? <p className="graph-warning">Inherited from {projectPath(projects, selected.project_id)}. <a href={`/graph?project=${encodeURIComponent(projects.find(p => p.id === selected.project_id)?.ref ?? "all")}&root=${encodeURIComponent(selected.id)}`}>Open the owning project to edit.</a></p>
              : <NodeOrganization key={`${selected.id}:${selected.updated_at}`} node={selected} projects={projects} />}
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
            {(payload.cross_project_edges ?? []).some(e => e.source_id === selected.id || e.target_id === selected.id) && <section className="detail-section"><h3>Cross-project references</h3>
              {(payload.cross_project_edges ?? []).filter(e => e.source_id === selected.id || e.target_id === selected.id).map(edge => {
                const other = payload.related_nodes?.find(n => n.id === (edge.source_id === selected.id ? edge.target_id : edge.source_id));
                return <p key={edge.id}><b>{edge.edge_type.replaceAll("_", " ")}{edge.metadata.proposed ? " (proposed)" : ""}</b> · {other && <a href={`/graph?project=${encodeURIComponent(projects.find(p => p.id === other.project_id)?.ref ?? "all")}&root=${encodeURIComponent(other.id)}`}>{other.title} — {projectPath(projects, other.project_id)}</a>}</p>;
              })}<p>References preserve context; they do not enable inheritance.</p>
            </section>}
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

export function GraphExplorer({ payload, initialFocusId = null }: { payload: GraphPayload; initialFocusId?: string | null }) {
  return <ReactFlowProvider><Explorer payload={payload} initialFocusId={initialFocusId} /></ReactFlowProvider>;
}
