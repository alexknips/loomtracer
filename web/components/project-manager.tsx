"use client";
import { useState } from "react";
import { useRouter } from "next/navigation";
import type { GraphEdgeRecord, GraphNodeRecord, ProjectRecord } from "@/types/graph";
import { ancestors, moveImpact, orderedProjects, projectPath } from "@/lib/projects";

export function ProjectManager({ projects, nodes, edges }: { projects: ProjectRecord[]; nodes: GraphNodeRecord[]; edges: GraphEdgeRecord[] }) {
  const router = useRouter();
  const [editing, setEditing] = useState<ProjectRecord | null>(null);
  const [name, setName] = useState("");
  const [ref, setRef] = useState("");
  const [description, setDescription] = useState("");
  const [parent, setParent] = useState("");
  const [pending, setPending] = useState(false);
  const [message, setMessage] = useState("");
  const [impact, setImpact] = useState<ReturnType<typeof moveImpact> | null>(null);
  const byId = new Map(nodes.map(n => [n.id, n]));
  const cross = edges.filter(e => byId.get(e.source_id)?.project_id && byId.get(e.target_id)?.project_id
    && byId.get(e.source_id)?.project_id !== byId.get(e.target_id)?.project_id);

  function edit(project: ProjectRecord | null) {
    setEditing(project); setName(project?.name ?? ""); setRef(project?.ref ?? "");
    setDescription(project?.description ?? ""); setParent(project?.parent_id ?? ""); setImpact(null); setMessage("");
  }
  async function save(preview: boolean) {
    setPending(true); setMessage("");
    try {
      const response = await fetch(editing ? `/api/projects/${editing.id}` : "/api/projects", {
        method: editing ? "PATCH" : "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ name, ref: ref || `project:${name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "")}`,
          description, parent_id: parent || null, expected_updated_at: editing?.updated_at, preview }),
      });
      const body = await response.json();
      if (!response.ok) throw new Error(body.error ?? "Could not save project.");
      if (preview && editing) { setImpact(body.impact); return; }
      if (body.demo) setMessage("Demo only. Reload restores the sample projects.");
      else { edit(null); router.refresh(); }
    } catch (error) { setMessage(error instanceof Error ? error.message : "Could not save."); }
    finally { setPending(false); }
  }
  return <main className="content-page projects-page">
    <header className="page-heading"><div><span className="eyebrow">Workspace organization</span><h1>Projects</h1></div>
      <p>Give decisions a home. Share selected parent decisions with descendants when you are ready.</p></header>
    <div className="project-grid">
      <section className="settings-card project-tree" aria-label="Project hierarchy"><h2>Your projects</h2>
        {orderedProjects(projects).map(project => {
          const local = nodes.filter(n => n.project_id === project.id);
          return <article key={project.id} style={{ marginLeft: ancestors(projects, project.id).length * 22 }}>
            <div><a href={`/graph?project=${encodeURIComponent(project.ref)}`}><b>{project.name}</b></a>
              <small>{local.length} entries · {local.filter(n => n.type === "Decision" && n.status === "active" && n.inheritance_mode === "descendants").length} decisions shared with descendants</small></div>
            <button className="text-button" type="button" onClick={() => edit(project)} aria-label={`Edit ${project.name}`}>Edit</button>
          </article>;
        })}
        <a href="/graph?project=unassigned">Unassigned · {nodes.filter(n => !n.project_id).length} entries</a>
      </section>
      <form className="settings-card organization-form" onSubmit={event => { event.preventDefault(); void save(Boolean(editing)); }}>
        <h2>{editing ? `Edit ${editing.name}` : "Create a project"}</h2>
        <label><span>Project name</span><input value={name} required maxLength={120} onChange={e => { setName(e.target.value); setImpact(null); }} /></label>
        {!editing && <label><span>Project reference (optional)</span><input value={ref} onChange={e => setRef(e.target.value)} placeholder="project:my-project" /></label>}
        <label><span>Parent project</span><select value={parent} onChange={e => { setParent(e.target.value); setImpact(null); }}>
          <option value="">No parent — top-level project</option>
          {orderedProjects(projects).filter(p => !editing || (p.id !== editing.id && !ancestors(projects, p.id).some(a => a.id === editing.id)))
            .map(p => <option key={p.id} value={p.id}>{projectPath(projects, p.id)}</option>)}
        </select></label>
        <label><span>Description</span><textarea value={description} maxLength={4000} onChange={e => { setDescription(e.target.value); setImpact(null); }} /></label>
        <div className="detail-actions"><button className="button primary" disabled={pending || !name.trim()}>{editing ? "Preview changes" : "Create project"}</button>
          {editing && <button className="button secondary" type="button" onClick={() => edit(null)}>Cancel</button>}</div>
        {impact && <section className="move-preview"><h3>Review project changes</h3><p>New location: {parent ? `${projectPath(projects, parent)} / ` : ""}{name}</p>
          {impact.map(item => <div key={item.project.id}><b>{item.project.name}</b><p>{item.gained.length} inherited decisions gained · {item.lost.length} lost</p>
            {item.gained.map(n => <p key={n.id}>Gain: {n.title}</p>)}{item.lost.map(n => <p key={n.id}>Lose: {n.title}</p>)}</div>)}
          <p>Local records retain their owners. References stay connected. Agent credentials are unchanged.</p>
          <button className="button primary" type="button" disabled={pending} onClick={() => save(false)}>Apply project changes</button>
        </section>}
        {message && <p role="status">{message}</p>}
      </form>
    </div>
    <section className="settings-card project-relations"><h2>Cross-project references</h2><p>{cross.length} existing relationships cross owning projects. References do not enable inherited guidance.</p>
      {cross.map(edge => {
        const source = byId.get(edge.source_id)!; const target = byId.get(edge.target_id)!;
        return <article key={edge.id}><a href={`/graph?project=${encodeURIComponent(projects.find(p => p.id === source.project_id)?.ref ?? "all")}&root=${encodeURIComponent(source.id)}`}>{source.title}</a>
          <span>{projectPath(projects, source.project_id)} → {edge.edge_type.replaceAll("_", " ")}{edge.metadata.proposed ? " (proposed)" : ""} → {projectPath(projects, target.project_id)}</span>
          <a href={`/graph?project=${encodeURIComponent(projects.find(p => p.id === target.project_id)?.ref ?? "all")}&root=${encodeURIComponent(target.id)}`}>{target.title}</a></article>;
      })}
    </section>
  </main>;
}
