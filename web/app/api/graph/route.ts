import { NextRequest } from "next/server";
import { requireOwnerApi } from "@/lib/server/auth";
import { getGraphPayload } from "@/lib/server/data";
import { apiError, privateJson } from "@/lib/server/http";

export const dynamic = "force-dynamic";

export async function GET(request: NextRequest) {
  try {
    const owner = await requireOwnerApi();
    const depth = Number(request.nextUrl.searchParams.get("depth") ?? 2);
    const root = request.nextUrl.searchParams.get("root");
    const payload = await getGraphPayload(owner.workspaceId, root, { depth });
    return privateJson(payload);
  } catch (error) {
    return apiError(error);
  }
}
