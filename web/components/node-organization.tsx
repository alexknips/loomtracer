"use client";
import { useState } from "react";
import { useRouter } from "next/navigation";
import type { GraphNodeRecord, ProjectRecord } from "@/types/graph";
import { ancestors, orderedProjects, projectPath } from "@/lib/projects";

export function NodeOrganization({ node, projects }: { node: GraphNodeRecord; projects: ProjectRecord[] }) {
  const router = useRouter();
  const [project, setProject] = useState(node.project_id ?? "");
  const [inheritance, setInheritance] = useState(node.inheritance_mode ?? "local");
  const [pending, setPending] = useState(false);
  const [message, setMessage] = useState("");
  const affected = project ? projects.filter(p => ancestors(projects, p.id).some(a => a.id === project)) : [];
  async function save() {
    setPending(true); setMessage("");
    try {
      const response = await fetch(`/api/nodes/${encodeURIComponent(node.id)}/organization`, {
        method: "PATCH", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ project_id: project || null, inheritance_mode: inheritance, expected_updated_at: node.updated_at }),
      });
      const body = await response.json();
      if (!response.ok) throw new Error(body.error ?? "Could not save organization.");
      if (body.demo) setMessage("Demo preview saved for this form only; reload restores the fixture.");
      else { setMessage("Saved."); router.refresh(); }
    } catch (error) { setMessage(error instanceof Error ? error.message : "Could not save."); }
    finally { setPending(false); }
  }
  return <section className="detail-section organization-form"><h3>Project & inheritance</h3>
    <label><span>Owning project</span><select value={project} onChange={e => { setProject(e.target.value); setInheritance("local"); }}>
      <option value="">Unassigned</option>{orderedProjects(projects).map(p => <option key={p.id} value={p.id}>{projectPath(projects, p.id)}</option>)}
    </select></label>
    <label><span>Applies to</span><select value={inheritance} onChange={e => setInheritance(e.target.value as "local" | "descendants")}>
      <option value="local">This project only</option>
      <option value="descendants" disabled={!project || node.type !== "Decision" || node.status !== "active"}>This project and all descendants</option>
    </select></label>
    {inheritance === "descendants" && <p>Applies to {affected.length ? affected.map(p => p.name).join(", ") : "no current children"}, plus future descendants. Existing workflow and task restrictions still apply.</p>}
    {project !== (node.project_id ?? "") && <p>Moving this record keeps its relationships and resets inheritance to local. Cross-project relationships remain visible for review.</p>}
    <button className="button secondary" type="button" disabled={pending || (project === (node.project_id ?? "") && inheritance === (node.inheritance_mode ?? "local"))} onClick={save}>{pending ? "Saving…" : "Save organization"}</button>
    {message && <p role="status">{message}</p>}
  </section>;
}
