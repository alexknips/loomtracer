"use client";

import { useMemo, useRef, useState } from "react";
import Link from "next/link";
import { Background, Controls, Handle, MarkerType, Position, ReactFlow, type Node, type NodeProps } from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { filterPublicDemo, publicDemoEdges, publicDemoNodes, type PublicDemoKind, type PublicDemoNode } from "@/lib/public-demo";

type SampleFlowNode = Node<{ record: PublicDemoNode; choose: (id: string) => void; current: boolean }, "sample">;

function SampleCard({ data }: NodeProps<SampleFlowNode>) {
  const record = data.record;
  return <>
    <Handle type="target" position={Position.Left} id="left" className="public-demo-handle" />
    <Handle type="target" position={Position.Top} id="top" className="public-demo-handle" />
    <button className={`public-demo-card nodrag ${data.current ? "public-demo-card-selected" : ""}`} onClick={() => data.choose(record.id)} aria-pressed={data.current}>
      <span className="public-demo-card-top"><span>{record.kind}</span><span className={`public-demo-status public-status-${record.status.toLowerCase()}`}>{record.status}</span></span>
      <strong>{record.title}</strong><span className="public-demo-card-summary">{record.summary}</span><small>{record.stage}</small>
    </button>
    <Handle type="source" position={Position.Right} id="right" className="public-demo-handle" />
    <Handle type="source" position={Position.Bottom} id="bottom" className="public-demo-handle" />
  </>;
}

const nodeTypes = { sample: SampleCard };

export function PublicDemo() {
  const [query, setQuery] = useState("");
  const [kind, setKind] = useState<"all" | PublicDemoKind>("all");
  const [selectedId, setSelectedId] = useState("team-plan");
  const [view, setView] = useState<"graph" | "list">("graph");
  const detailRef = useRef<HTMLElement>(null);
  const visible = useMemo(() => filterPublicDemo(query, kind), [query, kind]);
  const selected = visible.find(node => node.id === selectedId) ?? visible[0];
  const visibleIds = new Set(visible.map(node => node.id));

  function choose(id: string) {
    setSelectedId(id);
    if (window.matchMedia("(max-width: 760px)").matches) {
      detailRef.current?.focus({ preventScroll: true });
      detailRef.current?.scrollIntoView({ block: "start", behavior: "instant" });
    }
  }

  const nodes: SampleFlowNode[] = visible.map(record => ({ id: record.id, type: "sample", position: record.position, data: { record, choose, current: selected?.id === record.id } }));
  const edges = publicDemoEdges.filter(edge => visibleIds.has(edge.source) && visibleIds.has(edge.target)).map(edge => ({
    ...edge, type: "smoothstep", markerEnd: { type: MarkerType.ArrowClosed, color: edge.label === "supports" ? "#2f5d4b" : "#929d91" },
    style: { stroke: edge.label === "supports" ? "#2f5d4b" : "#929d91", strokeWidth: edge.label === "supports" ? 2 : 1.2 },
    labelStyle: { fill: "#332B32", fontSize: 10 }, labelBgStyle: { fill: "#FDF8F4" }, labelBgPadding: [6, 4] as [number, number], labelBgBorderRadius: 4,
  }));

  return <>
    <section className="public-demo-intro public-container">
      <div><p className="public-eyebrow"><span /> One question. Two investigations.</p><h1>See the connections.</h1><p>Two agents explore different ways to price a collaboration app. A third connects their findings into a proposal to test a team plan.</p></div>
      <Link className="public-button public-button-dark" href="/graph">Try it with your work <span aria-hidden="true">↗</span></Link>
    </section>
    <div className="public-demo-notice public-container"><span className="public-demo-tag">Fictional pricing study</span><p>The scenario and findings are invented to show the workflow. This read-only sample is separate from every private workspace.</p></div>
    <section className="public-demo-workspace public-container" aria-label="Sample decision explorer">
      <div className="public-demo-toolbar">
        <label><span>Search the sample</span><input type="search" placeholder="Try “predictable”" value={query} onChange={event => setQuery(event.target.value)} /></label>
        <label><span>Record type</span><select value={kind} onChange={event => setKind(event.target.value as "all" | PublicDemoKind)}><option value="all">All types</option><option>Decision</option><option>Evidence</option><option>Attempt</option></select></label>
        <div className="public-demo-view" role="group" aria-label="Sample view"><button aria-pressed={view === "graph"} onClick={() => setView("graph")}>Graph</button><button aria-pressed={view === "list"} onClick={() => setView("list")}>List</button></div>
        <span className="public-demo-count" role="status">{visible.length} of {publicDemoNodes.length} records</span>
        {(query || kind !== "all") && <button className="public-reset" onClick={() => { setQuery(""); setKind("all"); }}>Clear filters</button>}
      </div>
      <div className={`public-demo-body public-demo-view-${view}`}>
        <div className="public-demo-records">
          {visible.length === 0 ? <div className="public-demo-empty"><h2>No matching records</h2><p>Try another search or clear the filters.</p></div> : <>
            <div className="public-demo-canvas" aria-label="Read-only graph">
              <ReactFlow key={visible.map(node => node.id).join(":")} nodes={nodes} edges={edges} nodeTypes={nodeTypes} nodesDraggable={false} nodesConnectable={false} nodesFocusable={false} edgesFocusable={false} elementsSelectable={false} fitView fitViewOptions={{ padding: 0.15 }} minZoom={0.5} maxZoom={1.3} panOnScroll={false} zoomOnScroll={false} preventScrolling={false} proOptions={{ hideAttribution: true }}>
                <Background color="rgb(51 43 50 / 15%)" gap={20} size={1} /><Controls showInteractive={false} />
              </ReactFlow>
              <span className="public-canvas-note">Select a record to see its reasoning.</span>
            </div>
            <div className="public-demo-list" aria-label="Sample records">{visible.map(record => <button key={record.id} className={`public-demo-list-item ${record.id === selected?.id ? "public-demo-list-selected" : ""}`} onClick={() => choose(record.id)} aria-pressed={record.id === selected?.id}><span className="public-demo-card-top"><span>{record.kind}</span><span className={`public-demo-status public-status-${record.status.toLowerCase()}`}>{record.status}</span></span><strong>{record.title}</strong><span>{record.summary}</span><small>{record.stage} <span aria-hidden="true">↗</span></small></button>)}</div>
          </>}
        </div>
        <section className="public-demo-detail" aria-label="Selected record details" ref={detailRef} tabIndex={-1}>
          {selected ? <><div className="public-demo-card-top"><span>{selected.kind}</span><span className={`public-demo-status public-status-${selected.status.toLowerCase()}`}>{selected.status}</span></div><h2>{selected.title}</h2><p className="public-demo-detail-summary">{selected.summary}</p><p>{selected.detail}</p>
            {selected.reconsider.length > 0 && <div className="public-demo-detail-section"><h3>Reconsider when</h3><ul>{selected.reconsider.map(condition => <li key={condition}>{condition}</li>)}</ul></div>}
            <div className="public-demo-detail-section"><h3>Connected reasoning</h3><ul className="public-demo-relations">{publicDemoEdges.filter(edge => edge.source === selected.id || edge.target === selected.id).map(edge => {
              const otherId = edge.source === selected.id ? edge.target : edge.source;
              const other = publicDemoNodes.find(node => node.id === otherId)!;
              const label = edge.target === selected.id ? edge.label === "informs" ? "informed by" : "supported by" : edge.label;
              return <li key={edge.id}><span>{label}</span><button onClick={() => { setQuery(""); setKind("all"); setSelectedId(otherId); }}>{other.title} <span aria-hidden="true">↗</span></button></li>;
            })}</ul></div>
            <div className="public-demo-detail-section public-demo-source"><h3>Source</h3><p>{selected.source}.</p><p>{selected.stage}</p></div>
          </> : <p>Select a record after clearing your filters to inspect its evidence and history.</p>}
        </section>
      </div>
    </section>
    <section className="public-demo-takeaway public-container"><span className="public-mini-label">Build on your explorations.</span><p>The next agent can follow both pricing branches, see the shared criteria, and understand why a team-plan test is proposed. The candidate keeps its supporting evidence in view for human review and further experiments.</p></section>
  </>;
}
