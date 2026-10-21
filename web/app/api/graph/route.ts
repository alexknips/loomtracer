import { NextRequest } from "next/server";
import { requireOwnerApi } from "@/lib/server/auth";
import { getGraphPayload } from "@/lib/server/data";
import { apiError, privateJson } from "@/lib/server/http";

export const dynamic = "force-dynamic";

export async function GET(request: NextRequest) {
  try {
    const owner = await requireOwnerApi();
    const depthParam = request.nextUrl.searchParams.get("depth");
    const depth = Number(depthParam ?? 1);
    const root = request.nextUrl.searchParams.get("root");
    const requestedView = request.nextUrl.searchParams.get("view");
    const view = requestedView === "neighborhood" || depthParam !== null ? "neighborhood" : "all";
    const payload = await getGraphPayload(owner.workspaceId, root, { view, depth, project: request.nextUrl.searchParams.get("project") ?? "all", includeInherited: request.nextUrl.searchParams.get("inherited") === "true" });
    return privateJson(payload);
  } catch (error) {
    return apiError(error);
  }
}
