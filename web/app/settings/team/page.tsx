import { AppShell } from "@/components/app-shell";
import { TeamManager } from "@/components/team-manager";
import { requireOwner } from "@/lib/server/auth";
import { createClient } from "@/lib/supabase/server";

export const dynamic = "force-dynamic";
export default async function TeamPage() {
  const context = await requireOwner();
  const db = context.demo ? null : await createClient();
  const members = db ? await db.rpc("list_workspace_members") : { data: [{ user_id: context.user.id, email: context.user.email, role: "owner" }], error: null };
  const invitations = db && context.membershipRole === "owner" ? await db.rpc("list_workspace_invitations") : { data: [], error: null };
  if (members.error || invitations.error) throw new Error("Could not load team membership. Please refresh.");
  return <AppShell active="Team"><TeamManager workspaceName={context.workspaceName} isOwner={context.membershipRole === "owner"} members={members.data ?? []} invitations={invitations.data ?? []} demo={context.demo} /></AppShell>;
}
