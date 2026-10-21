import { z } from "zod";
import { privateJson, requireSameOrigin } from "@/lib/server/http";
import { humanReasoning } from "@/lib/server/reasoning-context";
import { projectRefSchema, operationSchema, readReasoningJson, reasoningError } from "@/lib/server/reasoning-http";
export const dynamic = "force-dynamic";
const input = z.object({ project_ref: projectRefSchema, dataset_id: z.string().min(1), action: z.enum(["adopt", "revise", "withdraw"]), entry_id: z.string().min(1), prior_receipt_id: z.string().nullable(), rationale: z.string().trim().min(1).max(20000), expected_head: z.string().min(1), operation_id: operationSchema }).strict();
export async function POST(request: Request) {
  try {
    requireSameOrigin(request);
    const service = await humanReasoning(), a = input.parse(await readReasoningJson(request));
    return privateJson(await service.commit(a.project_ref, a.dataset_id, a.action, a.entry_id, a.prior_receipt_id, a.rationale, a.expected_head, a.operation_id));
  } catch (error) { return reasoningError(error); }
}
