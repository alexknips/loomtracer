import { getOwnerContext } from "@/lib/server/auth";
import { privateJson } from "@/lib/server/http";
export async function GET() {
  const owner = await getOwnerContext();
  return owner ? privateJson({ userId: owner.user.id, workspaceId: owner.workspaceId, membershipRole: owner.membershipRole }) : privateJson({ error: "Unauthorized" }, { status: 401 });
}
