import type { GraphNodeRecord, GraphPayload, ProjectRecord } from "@/types/graph";

export const PROJECT_REF = /^project:[a-z0-9][a-z0-9._/-]*$/;
export const PROJECT_COOKIE = "neurofilament-project";

export function readProjectCookie(value?: string): string {
  try { return value ? decodeURIComponent(value) : "all"; }
  catch { return "all"; }
}

export function resolveProject(projects: ProjectRecord[], value: string) {
  return projects.find(project => project.id === value || project.ref === value);
}

// Closest parent first. Invalid hierarchies fail visibly instead of hiding guidance.
export function ancestors(projects: ProjectRecord[], id: string): ProjectRecord[] {
  const byId = new Map(projects.map(project => [project.id, project]));
  const start = byId.get(id);
  if (!start) throw new Error("Project not found.");
  const seen = new Set([id]);
  const result: ProjectRecord[] = [];
  let parentId = start.parent_id;
  while (parentId) {
    if (seen.has(parentId)) throw new Error("Project hierarchy contains a cycle.");
    seen.add(parentId);
    const parent = byId.get(parentId);
    if (!parent || parent.workspace_id !== start.workspace_id) throw new Error("Project parent is unavailable.");
    result.push(parent);
    parentId = parent.parent_id;
  }
  return result;
}

export function projectPath(projects: ProjectRecord[], id?: string | null): string {
  const project = projects.find(p => p.id === id);
  return project ? [...ancestors(projects, project.id).reverse(), project].map(p => p.name).join(" / ") : "Unassigned";
}

export function orderedProjects(projects: ProjectRecord[]) {
  return [...projects].sort((a, b) => projectPath(projects, a.id).localeCompare(projectPath(projects, b.id)));
}

export function inheritedNodes(nodes: GraphNodeRecord[], projects: ProjectRecord[], projectId: string) {
  const project = projects.find(p => p.id === projectId);
  if (!project) throw new Error("Project not found.");
  const ids = new Set(ancestors(projects, projectId).map(p => p.id));
  return nodes.filter(n => n.workspace_id === project.workspace_id && n.project_id && ids.has(n.project_id)
    && n.type === "Decision" && n.status === "active" && n.inheritance_mode === "descendants")
    .map(n => ({ ...n, inherited_from_project_id: n.project_id! }));
}

export function selectProjectNodes(nodes: GraphNodeRecord[], projects: ProjectRecord[], filter = "all", includeInherited = false) {
  if (filter === "all") return nodes;
  if (filter === "unassigned") return nodes.filter(n => !n.project_id);
  const project = resolveProject(projects, filter);
  if (!project) throw new Error("Project not found. Choose an existing project.");
  const local = nodes.filter(n => n.workspace_id === project.workspace_id && n.project_id === project.id);
  return includeInherited ? [...local, ...inheritedNodes(nodes, projects, project.id)] : local;
}

export function projectGraph(payload: GraphPayload, filter = "all", options: {
  includeInherited?: boolean; root?: string | null; view?: "all" | "neighborhood"; depth?: number; maxNodes?: number; maxEdges?: number;
} = {}): GraphPayload {
  const projects = payload.projects ?? [];
  const project = resolveProject(projects, filter);
  const eligible = selectProjectNodes(payload.nodes, projects, filter, options.includeInherited);
  const eligibleIds = new Set(eligible.map(n => n.id));
  const eligibleEdges = payload.edges.filter(e => eligibleIds.has(e.source_id) && eligibleIds.has(e.target_id));
  const view = options.view ?? "all";
  const depth = Number.isFinite(options.depth) ? Math.min(5, Math.max(0, Math.floor(options.depth!))) : 1;
  const root = options.root && eligibleIds.has(options.root) ? options.root : eligible[0]?.id ?? null;
  let candidates = eligible;
  if (view === "neighborhood") {
    const visible = new Set(root ? [root] : []);
    let frontier = new Set(visible);
    for (let hop = 0; hop < depth; hop++) {
      const next = new Set<string>();
      for (const e of eligibleEdges) {
        if (frontier.has(e.source_id) && !visible.has(e.target_id)) next.add(e.target_id);
        if (frontier.has(e.target_id) && !visible.has(e.source_id)) next.add(e.source_id);
      }
      for (const id of next) visible.add(id);
      frontier = next;
    }
    candidates = eligible.filter(n => visible.has(n.id));
  }
  // Keep a requested focus in a bounded all-project canvas.
  if (root) candidates = [...candidates.filter(n => n.id === root), ...candidates.filter(n => n.id !== root)];
  const nodes = candidates.slice(0, options.maxNodes ?? 500);
  const ids = new Set(nodes.map(n => n.id));
  const matchingEdges = eligibleEdges.filter(e => ids.has(e.source_id) && ids.has(e.target_id));
  const edges = matchingEdges.slice(0, options.maxEdges ?? 1000);
  const localIds = new Set(selectProjectNodes(payload.nodes, projects, filter).map(n => n.id));
  const cross = payload.edges.filter(e => {
    const source = payload.nodes.find(n => n.id === e.source_id);
    const target = payload.nodes.find(n => n.id === e.target_id);
    return source?.project_id && target?.project_id && source.project_id !== target.project_id
      && (filter === "all" || localIds.has(e.source_id) || localIds.has(e.target_id));
  });
  const relatedIds = new Set(cross.flatMap(e => [e.source_id, e.target_id]));
  return { ...payload, nodes, edges, root_id: root, view, depth,
    total_nodes: eligible.length, total_edges: eligibleEdges.length,
    truncated: candidates.length > nodes.length || matchingEdges.length > edges.length,
    project_filter: project?.ref ?? filter, include_inherited: options.includeInherited ?? false,
    inherited_nodes: project ? inheritedNodes(payload.nodes, projects, project.id) : [],
    cross_project_edges: cross, related_nodes: payload.nodes.filter(n => relatedIds.has(n.id)),
    audits: Object.fromEntries(Object.entries(payload.audits).filter(([id]) => ids.has(id))),
  };
}

export function moveImpact(projects: ProjectRecord[], nodes: GraphNodeRecord[], projectId: string, parentId: string | null) {
  const before = projects.find(p => p.id === projectId);
  if (!before) throw new Error("Project not found.");
  const after = projects.map(p => p.id === projectId ? { ...p, parent_id: parentId } : p);
  ancestors(after, projectId);
  const affected = projects.filter(p => p.id === projectId || ancestors(projects, p.id).some(a => a.id === projectId));
  return affected.map(p => {
    const oldNodes = inheritedNodes(nodes, projects, p.id);
    const newNodes = inheritedNodes(nodes, after, p.id);
    return { project: p, gained: newNodes.filter(n => !oldNodes.some(o => o.id === n.id)), lost: oldNodes.filter(n => !newNodes.some(o => o.id === n.id)) };
  });
}
