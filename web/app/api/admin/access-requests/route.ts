import { requireOwnerApi } from "@/lib/server/auth";
import { createClient } from "@/lib/supabase/server";
import { apiError, privateJson } from "@/lib/server/http";

export async function GET() {
  try {
    const context = await requireOwnerApi();
    if (!context.isAdmin) throw new Error("FORBIDDEN");
    if (context.demo) return privateJson({ requests: [] });
    const { data, error } = await (await createClient()).rpc("list_access_requests");
    if (error) throw error;
    return privateJson({ requests: data });
  } catch (error) { return apiError(error); }
}
