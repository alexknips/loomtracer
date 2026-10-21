import type { DecisionStatus, GraphEdge, ProjectEntry, ProjectSummary } from "./types";

/** The project a decision is read under when the graph names none (a decision only referenced, never proposed). The underscore cannot occur in a project handle (letters, digits and dashes). */
export const UNASSIGNED = "_unassigned";
export const STATUSES: DecisionStatus[] = ["proposed", "accepted", "contested", "rejected", "superseded"];

/** The edge as the graph stores it: the side making the claim first. A reversed arrow runs the other way. */
export function stored(edge: GraphEdge): { source: string; target: string } {
  return edge.reversed ? { source: edge.to, target: edge.from } : { source: edge.from, target: edge.to };
}
export const nodeRawId = (nodeId: string) => nodeId.slice(nodeId.indexOf(":") + 1);
export const nodeKind = (nodeId: string) => nodeId.slice(0, nodeId.indexOf(":"));
export const projectOf = (decision: { project: string | null }) => decision.project ?? UNASSIGNED;

/** The projects of `GET /v1/projects` as a tree: each with its parent, its children and its decisions by status, in path order. */
export function summarizeProjects(entries: ProjectEntry[]): ProjectSummary[] {
  const byRef = new Map<string, ProjectSummary>(entries.map(entry => [entry.handle, { ref: entry.handle, name: entry.name || entry.handle, parent: null, children: [], counts: { ...entry.decisions } }]));
  for (const entry of entries) {
    const project = byRef.get(entry.handle)!, parent = entry.parent ? byRef.get(entry.parent) : undefined;
    if (parent && parent !== project) { project.parent = parent.ref; parent.children.push(project.ref); }
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
