"use client";
import { useRouter } from "next/navigation";
import { useEffect } from "react";
import { orderedProjects, projectPath, PROJECT_COOKIE } from "@/lib/projects";
import type { ProjectRecord } from "@/types/graph";

export function ProjectSelector({ projects, value = "all", destination = "/graph" }: { projects: ProjectRecord[]; value?: string; destination?: string }) {
  const router = useRouter();
  useEffect(() => {
    document.cookie = `${PROJECT_COOKIE}=${encodeURIComponent(value)}; Path=/; Max-Age=31536000; SameSite=Lax`;
  }, [value]);
  return <label className="project-selector"><span>Project</span>
    <select value={value} onChange={event => {
      const project = event.target.value;
      document.cookie = `${PROJECT_COOKIE}=${encodeURIComponent(project)}; Path=/; Max-Age=31536000; SameSite=Lax`;
      router.push(`${destination}?project=${encodeURIComponent(project)}`);
    }}>
      <option value="all">All projects</option><option value="unassigned">Unassigned</option>
      {orderedProjects(projects).map(p => <option key={p.id} value={p.ref}>{projectPath(projects, p.id)}</option>)}
    </select>
  </label>;
}
