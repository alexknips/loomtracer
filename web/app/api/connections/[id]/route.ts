import { z } from "zod";
import { requireOwnerApi } from "@/lib/server/auth";
import { createClient } from "@/lib/supabase/server";
import { apiError, privateJson, requireSameOrigin } from "@/lib/server/http";
export async function DELETE(request: Request, { params }: { params: Promise<{ id: string }> }) {
  try {
    requireSameOrigin(request); await requireOwnerApi();
    const id = z.uuid().parse((await params).id);
    const supabase = await createClient();
    const { data: clientId, error } = await supabase.rpc("disconnect_agent", { connection_id: id });
    if (error) throw error;
    const { error: revokeError } = await supabase.auth.oauth.revokeGrant({ clientId });
    return privateJson({ ok: true, cleanupPending: Boolean(revokeError) });
  } catch (error) { return apiError(error); }
}
