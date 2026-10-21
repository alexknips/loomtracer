import type { Metadata } from "next";
import Link from "next/link";
import { AppShell } from "@/components/app-shell";
import { HiveMindProblem } from "@/components/hivemind-problem";
import { StatusBadge } from "@/components/status-badge";
import { hivemind } from "@/lib/hivemind/client";
import { load } from "@/lib/hivemind/load";
import { listProjects, STATUSES, UNASSIGNED } from "@/lib/hivemind/projects";

export const dynamic = "force-dynamic";
export const metadata: Metadata = { title: "Projects · Loomtracer" };

export default async function ProjectsPage() {
  const result = await load(async () => listProjects(await hivemind().graph()));
  if (!result.ok) return <AppShell active="Projects"><HiveMindProblem error={result.error} /></AppShell>;
  const projects = result.value;
  const byRef = new Map(projects.map(project => [project.ref, project]));
  const depth = (ref: string) => { let count = 0; for (let up = byRef.get(ref)?.parent; up && count < 3; up = byRef.get(up)?.parent) count += 1; return count; };
  return <AppShell active="Projects"><section className="content-page">
    <header className="page-heading"><div><span className="eyebrow">Projects</span><h1>Where decisions live</h1></div><p>Every decision is filed under one project. A project that is part of another sits beneath it.</p></header>
    {projects.length ? <div className="hm-projects" data-testid="project-list">{projects.map(project => <article className="hm-project" key={project.ref} data-depth={depth(project.ref)} data-testid="project">
      <div>
        <h2><Link href={`/graph?${new URLSearchParams({ project: project.ref })}`}>{project.name}</Link></h2>
        <p>{project.ref === UNASSIGNED ? "Decisions filed under no project" : project.ref} · {project.counts.total} {project.counts.total === 1 ? "decision" : "decisions"}{project.parent ? <> · part of {byRef.get(project.parent)?.name}</> : null}</p>
      </div>
      <div className="hm-counts">{STATUSES.filter(status => project.counts[status] > 0).map(status => <span key={status}><StatusBadge status={status} /> {project.counts[status]}</span>)}</div>
    </article>)}</div> : <p className="hm-empty">This ledger holds no projects and no decisions yet. Capture one with <code>hivemind emit decision.capture</code>.</p>}
    <p className="hm-note">Projects are registered and linked with <code>hivemind project</code>. The HiveMind HTTP API has no endpoint to create, move or link one, so this page only lists them.</p>
  </section></AppShell>;
}
