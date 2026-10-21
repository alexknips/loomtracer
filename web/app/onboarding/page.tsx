import { AppShell } from "@/components/app-shell";
import { requireOwner } from "@/lib/server/auth";
import { ProjectManager } from "@/components/project-manager";
import { createClient } from "@/lib/supabase/server";

export const dynamic = "force-dynamic";
export default async function OnboardingPage() {
  const member = await requireOwner();
  const { data, error } = member.demo ? { data: [], error: null }
    : await (await createClient()).from("projects").select("id").eq("workspace_id", member.workspaceId).limit(1);
  if (error) throw new Error("Could not load projects.");
  return <AppShell active="Projects"><section className="content-page">
    <span className="eyebrow">Getting started · {member.workspaceName}</span><h1>Your workspace is ready.</h1>
    <p>{member.membershipRole === "owner" ? "You own this workspace. It is separate from other teams’ workspaces, and you manage invitations in Team settings." : "You joined this workspace as a member. You can review decisions and manage projects with your teammates."} Each person authorizes their own agents.</p>
    <ol><li>Sign in with your invited Google account ✓</li><li>{data?.length ? "Your workspace has a project ✓" : "Create the first project"}</li><li>Connect Codex or Claude Code</li><li>Verify your agent’s first project briefing</li></ol>
    {data?.length ? <a className="button primary" href="/settings/connections">Connect my agent</a>
      : <p>Give your first project a name below.</p>}
  </section>
    {!data?.length && <ProjectManager projects={[]} nodes={[]} edges={[]} />}
  </AppShell>;
}
