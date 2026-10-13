import { z } from "zod";
import { requireOwnerApi } from "@/lib/server/auth";
import { createClient } from "@/lib/supabase/server";
import { privateJson, requireSameOrigin } from "@/lib/server/http";
import { invitationError, invitationLink, invitedEmailSchema } from "@/lib/server/invitations";

export async function POST(request: Request) {
  try {
    requireSameOrigin(request);
    const context = await requireOwnerApi();
    if (context.membershipRole !== "owner" || context.demo) throw new Error("FORBIDDEN");
    const { email } = z.object({ email: invitedEmailSchema }).parse(await request.json());
    const { data, error } = await (await createClient()).rpc("create_workspace_invitation", { invited_email: email });
    if (error) throw error;
    return privateJson({ id: data.id, email: data.email, link: invitationLink(data) });
  } catch (error) { return invitationError(error); }
}
