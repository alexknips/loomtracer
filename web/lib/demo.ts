import chronicleGraph from "@/lib/project-chronicle-graph.json" with { type: "json" };
import type { GraphPayload, ReviewRecord, ProjectRecord, GraphNodeRecord } from "@/types/graph";

const workspace = chronicleGraph.nodes[0].workspace_id;
const now = "2026-09-11T00:00:00Z";
export const demoProjects: ProjectRecord[] = [
  ["10000000-0000-4000-8000-000000000001", "project:neurofilament", "Neurofilament", null],
  ["10000000-0000-4000-8000-000000000002", "project:worldspring", "Worldspring", null],
  ["10000000-0000-4000-8000-000000000003", "project:echeasea", "Echeasea", null],
  ["10000000-0000-4000-8000-000000000004", "project:echeasea/hush", "Hush", "10000000-0000-4000-8000-000000000003"],
  ["10000000-0000-4000-8000-000000000005", "project:echeasea/haunted-manor", "Haunted Manor", "10000000-0000-4000-8000-000000000003"],
].map(([id, ref, name, parent_id]) => ({ id: id!, ref: ref!, name: name!, parent_id, workspace_id: workspace, description: "", created_at: now, updated_at: now }));
const example = chronicleGraph.nodes[0] as GraphNodeRecord;
const demoContent: GraphNodeRecord[] = [
  { ...example, id: "D-demo-setting", type: "Decision", status: "active", title: "Preserve player choice across Echeasea adventures", summary: "Demo inherited guidance: offer meaningful alternatives.", body: "Demo fixture only.", scope_refs: [], project_id: demoProjects[2].id, inheritance_mode: "descendants", metadata: {} },
  { ...example, id: "D-demo-hush", type: "Decision", status: "active", title: "Hush keeps its peaceful resolution", summary: "Demo local decision.", body: "Demo fixture only.", scope_refs: ["artifact:hush"], project_id: demoProjects[3].id, inheritance_mode: "local", metadata: {} },
  { ...example, id: "D-demo-manor", type: "Decision", status: "candidate", title: "Haunted Manor opens with an invitation", summary: "Demo candidate awaiting owner review.", body: "Demo fixture only.", scope_refs: ["artifact:haunted-manor"], project_id: demoProjects[4].id, inheritance_mode: "local", metadata: {} },
];
export const demoGraph: GraphPayload = { ...(chronicleGraph as GraphPayload), projects: demoProjects,
  nodes: [...chronicleGraph.nodes.map(n => ({ ...n, project_id: demoProjects[0].id, inheritance_mode: "local" }) as GraphNodeRecord), ...demoContent] };

export function getDemoGraph(
  requestedRoot?: string | null,
  options: { view?: "all" | "neighborhood"; depth?: number; maxNodes?: number; maxEdges?: number } = {},
): GraphPayload {
  const rootId = requestedRoot && demoGraph.nodes.some((node) => node.id === requestedRoot)
    ? requestedRoot
    : demoGraph.root_id;
  if (!rootId) return { root_id: null, nodes: [], edges: [], audits: {}, truncated: false, view: "all", total_nodes: 0, total_edges: 0 };

  const view = options.view ?? (options.depth === undefined ? "all" : "neighborhood");
  if (view === "all") {
    return {
      ...demoGraph,
      root_id: rootId,
      view,
      depth: undefined,
      total_nodes: demoGraph.nodes.length,
      total_edges: demoGraph.edges.length,
    };
  }

  const depth = Math.min(Math.max(options.depth ?? demoGraph.depth ?? 2, 0), 5);
  const maxNodes = Math.min(Math.max(options.maxNodes ?? 200, 1), 200);
  const maxEdges = Math.min(Math.max(options.maxEdges ?? 400, 0), 400);
  const visible = new Set<string>([rootId]);
  let frontier = [rootId];

  for (let hop = 0; hop < depth && frontier.length > 0 && visible.size < maxNodes; hop += 1) {
    const next: string[] = [];
    for (const currentId of frontier) {
      for (const edge of demoGraph.edges) {
        const other = edge.source_id === currentId
          ? edge.target_id
          : edge.target_id === currentId
            ? edge.source_id
            : null;
        if (!other || visible.has(other)) continue;
        visible.add(other);
        next.push(other);
        if (visible.size >= maxNodes) break;
      }
      if (visible.size >= maxNodes) break;
    }
    frontier = next;
  }

  const nodes = demoGraph.nodes.filter((node) => visible.has(node.id)).slice(0, maxNodes);
  const retainedIds = new Set(nodes.map((node) => node.id));
  const matchingEdges = demoGraph.edges.filter(
    (edge) => retainedIds.has(edge.source_id) && retainedIds.has(edge.target_id),
  );
  const edges = matchingEdges.slice(0, maxEdges);
  const audits = Object.fromEntries(
    Object.entries(demoGraph.audits).filter(([nodeId]) => retainedIds.has(nodeId)),
  );
  return {
    root_id: rootId,
    nodes,
    edges,
    audits,
    view,
    depth,
    total_nodes: demoGraph.nodes.length,
    total_edges: demoGraph.edges.length,
    truncated: visible.size > maxNodes || matchingEdges.length > maxEdges,
  };
}

export const demoReview: ReviewRecord[] = demoGraph.nodes
  .filter((node) => (node.type === "Decision" && node.status === "candidate")
    || (node.type === "Assumption" && node.status === "proposed"))
  .map((node) => ({
    ...node,
    proposed_supersedes: typeof node.metadata.proposed_supersedes === "string"
      ? demoGraph.nodes.find((candidate) => candidate.id === node.metadata.proposed_supersedes) ?? null
      : null,
  }));
