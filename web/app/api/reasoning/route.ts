import { z } from "zod";
import { privateJson, requireSameOrigin } from "@/lib/server/http";
import { humanReasoning } from "@/lib/server/reasoning-context";
import { projectRefSchema, operationSchema, headSchema, readReasoningJson, reasoningError } from "@/lib/server/reasoning-http";
export const dynamic = "force-dynamic";
export async function GET(request: Request) {
  try {
    const service = await humanReasoning(), params = new URL(request.url).searchParams;
    if (!params.has("project_ref")) return privateJson({ projects: await service.projects(), actor: service.identity.actor, isOwner: service.identity.isOwner });
    const ref = projectRefSchema.parse(params.get("project_ref")), id = params.get("dataset_id");
    return privateJson(id ? await service.read(ref, id) : await service.datasets(ref));
  } catch (error) { return reasoningError(error); }
}
const input = z.object({ project_ref: projectRefSchema, bundle: z.unknown(), expected_head: headSchema, operation_id: operationSchema }).strict();
export async function POST(request: Request) {
  try {
    requireSameOrigin(request);
    const service = await humanReasoning(), args = input.parse(await readReasoningJson(request));
    return privateJson(await service.save(args.project_ref, args.bundle, args.expected_head, args.operation_id));
  } catch (error) { return reasoningError(error); }
}
