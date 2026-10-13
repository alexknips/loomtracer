import { requireOwnerApi } from "@/lib/server/auth";
import { createClient } from "@/lib/supabase/server";
import { apiError, privateJson } from "@/lib/server/http";

export async function GET() {
  try {
    const context = await requireOwnerApi();
    if (context.demo) return privateJson({ members: [{ user_id: context.user.id, email: context.user.email, role: "owner" }], invitations: [] });
    const db = await createClient();
    const [members, invitations] = await Promise.all([
      db.rpc("list_workspace_members"),
      context.membershipRole === "owner" ? db.rpc("list_workspace_invitations") : Promise.resolve({ data: [], error: null }),
    ]);
    if (members.error || invitations.error) throw members.error || invitations.error;
    return privateJson({ members: members.data, invitations: invitations.data });
  } catch (error) { return apiError(error); }
}
