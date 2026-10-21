import { z } from "zod";
import { requireOwnerApi } from "@/lib/server/auth";
import { createClient } from "@/lib/supabase/server";
import { apiError, privateJson, requireSameOrigin } from "@/lib/server/http";
export async function PATCH(request: Request, { params }: { params: Promise<{ id: string }> }) {
  try {
    requireSameOrigin(request);
    const context = await requireOwnerApi();
    if (!context.isAdmin || context.demo) throw new Error("FORBIDDEN");
    const id = z.uuid().parse((await params).id);
    const { status } = z.object({ status: z.enum(["active", "suspended"]) }).parse(await request.json());
    const { error } = await (await createClient()).rpc("set_account_status", { target_user_id: id, new_status: status });
    if (error) throw error;
    return privateJson({ ok: true });
  } catch (error) { return apiError(error); }
}
