import type { DecisionStatus, GraphEdge, GraphResponse, ProjectSummary } from "./types";

/** Decisions filed under no project. The underscore cannot occur in a project handle (letters, digits and dashes). */
export const UNASSIGNED = "_unassigned";
export const STATUSES: DecisionStatus[] = ["proposed", "accepted", "contested", "rejected", "superseded"];

/** The edge as the graph stores it: the side making the claim first. A reversed arrow runs the other way. */
export function stored(edge: GraphEdge): { source: string; target: string } {
  return edge.reversed ? { source: edge.to, target: edge.from } : { source: edge.from, target: edge.to };
}
export const nodeRawId = (nodeId: string) => nodeId.slice(nodeId.indexOf(":") + 1);
export const nodeKind = (nodeId: string) => nodeId.slice(0, nodeId.indexOf(":"));
export const projectOf = (decision: { project: string | null }) => decision.project ?? UNASSIGNED;

export function listProjects(graph: GraphResponse): ProjectSummary[] {
  const byRef = new Map<string, ProjectSummary>();
  const ensure = (ref: string, name?: string | null) => {
    let project = byRef.get(ref);
    if (!project) {
      project = { ref, name: name || (ref === UNASSIGNED ? "Unassigned" : ref), parent: null, children: [], counts: { proposed: 0, accepted: 0, contested: 0, rejected: 0, superseded: 0, total: 0 } };
      byRef.set(ref, project);
    } else if (name && project.name === ref) project.name = name;
    return project;
  };
  for (const node of graph.nodes) if (node.kind === "Project") ensure(nodeRawId(node.id), node.label);
  for (const decision of graph.decisions) {
    const counts = ensure(projectOf(decision)).counts;
    counts[decision.status] += 1; counts.total += 1;
  }
  for (const edge of graph.edges) {
    if (edge.relation !== "PART_OF") continue;
    const { source, target } = stored(edge);
    const child = byRef.get(nodeRawId(source)), parent = byRef.get(nodeRawId(target));
    if (child && parent && child.parent === null && child !== parent) { child.parent = parent.ref; parent.children.push(child.ref); }
  }
  // A cycle would hide the whole ring from the tree below; break it at the first project met.
  for (const project of byRef.values()) {
    const seen = new Set([project.ref]);
    for (let up = project.parent ? byRef.get(project.parent) : undefined; up; up = up.parent ? byRef.get(up.parent) : undefined) {
      if (seen.has(up.ref)) { const parent = byRef.get(project.parent!); if (parent) parent.children = parent.children.filter(ref => ref !== project.ref); project.parent = null; break; }
      seen.add(up.ref);
    }
  }
  return [...byRef.values()].sort((a, b) => pathOf(byRef, a.ref).localeCompare(pathOf(byRef, b.ref)));
}

function pathOf(projects: Map<string, ProjectSummary>, ref: string): string {
  const names: string[] = [];
  const seen = new Set<string>();
  for (let project = projects.get(ref); project && !seen.has(project.ref); project = project.parent ? projects.get(project.parent) : undefined) { seen.add(project.ref); names.unshift(project.name); }
  return names.join(" / ");
}
export const projectPath = (projects: ProjectSummary[], ref: string) => pathOf(new Map(projects.map(project => [project.ref, project])), ref);
