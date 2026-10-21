import { z } from "zod";
import { requireOwnerApi } from "@/lib/server/auth";
import { createClient } from "@/lib/supabase/server";
import { apiError, privateJson, requireSameOrigin } from "@/lib/server/http";

export async function DELETE(request: Request, { params }: { params: Promise<{ id: string }> }) {
  try {
    requireSameOrigin(request);
    const context = await requireOwnerApi();
    if (context.membershipRole !== "owner" || context.demo) throw new Error("FORBIDDEN");
    const id = z.uuid().parse((await params).id);
    const { error } = await (await createClient()).rpc("revoke_workspace_invitation", { invitation_id: id });
    if (error) throw error;
    return privateJson({ ok: true });
  } catch (error) { return apiError(error); }
}
