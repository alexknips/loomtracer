import { z } from "zod";
import { privateJson, requireSameOrigin } from "@/lib/server/http";
import { humanReasoning } from "@/lib/server/reasoning-context";
import { projectRefSchema, readReasoningJson, reasoningError } from "@/lib/server/reasoning-http";
export const dynamic = "force-dynamic";
export async function POST(request: Request) {
  try {
    requireSameOrigin(request);
    const service = await humanReasoning();
    const a = z.object({ project_ref: projectRefSchema, connection_id: z.string().uuid() }).strict().parse(await readReasoningJson(request));
    return privateJson({ grant: await service.grant(a.project_ref, a.connection_id) });
  } catch (error) { return reasoningError(error); }
}
export async function DELETE(request: Request) {
  try {
    requireSameOrigin(request);
    const service = await humanReasoning();
    const a = z.object({ project_ref: projectRefSchema, grant_id: z.string().uuid() }).strict().parse(await readReasoningJson(request));
    return privateJson({ grant: await service.revoke(a.project_ref, a.grant_id) });
  } catch (error) { return reasoningError(error); }
}
