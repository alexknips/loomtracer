import { z } from "zod";
import { invitationError, invitationLink, invitedEmailSchema, workspaceNameSchema } from "@/lib/server/invitations";
import { requireOwnerApi } from "@/lib/server/auth";
import { createClient } from "@/lib/supabase/server";
import { apiError, privateJson, requireSameOrigin } from "@/lib/server/http";
export async function GET() {
  try {
    const owner = await requireOwnerApi();
    if (!owner.isAdmin) throw new Error("FORBIDDEN");
    if (owner.demo) return privateJson({ enabled: false, invitations: [], accounts: [] });
    const { data, error } = await (await createClient()).rpc("list_beta_invitations");
    if (error) throw error;
    return privateJson(data);
  } catch (error) { return apiError(error); }
}
export async function POST(request: Request) {
  try {
    requireSameOrigin(request);
    const owner = await requireOwnerApi();
    if (!owner.isAdmin || owner.demo) throw new Error("FORBIDDEN");
    const { email, workspaceName } = z.object({ email: invitedEmailSchema, workspaceName: workspaceNameSchema }).parse(await request.json());
    const { data, error } = await (await createClient()).rpc("create_beta_invitation", { invited_email: email, invited_workspace_name: workspaceName });
    if (error) throw error;
    const link = invitationLink(data);
    return privateJson({ id: data.id, email: data.email, workspace_name: data.workspace_name, kind: data.kind, expires_at: data.expires_at, link });
  } catch (error) { return invitationError(error); }
}
