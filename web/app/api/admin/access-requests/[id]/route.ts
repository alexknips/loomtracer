import { z } from "zod";
import { requireOwnerApi } from "@/lib/server/auth";
import { createClient } from "@/lib/supabase/server";
import { privateJson, requireSameOrigin } from "@/lib/server/http";
import { invitationError, invitationLink, workspaceNameSchema } from "@/lib/server/invitations";

export async function PATCH(request: Request, { params }: { params: Promise<{ id: string }> }) {
  try {
    requireSameOrigin(request);
    const context = await requireOwnerApi();
    if (!context.isAdmin || context.demo) throw new Error("FORBIDDEN");
    const id = z.uuid().parse((await params).id);
    const input = z.discriminatedUnion("action", [
      z.object({ action: z.literal("approve"), workspaceName: workspaceNameSchema }),
      z.object({ action: z.literal("dismiss") }),
    ]).parse(await request.json());
    const { data, error } = input.action === "approve"
      ? await (await createClient()).rpc("approve_access_request", { request_id: id, invited_workspace_name: input.workspaceName })
      : await (await createClient()).rpc("dismiss_access_request", { request_id: id });
    if (error) throw error;
    return privateJson(input.action === "approve" ? { ok: true, email: data.email, workspace_name: data.workspace_name, link: invitationLink(data) } : { ok: true });
  } catch (error) { return invitationError(error); }
}
