import { NextRequest } from "next/server";
import { demoGraph } from "@/lib/demo";
import { isDemoMode } from "@/lib/env";
import { requireOwnerApi } from "@/lib/server/auth";
import { apiError, privateJson } from "@/lib/server/http";
import { createClient } from "@/lib/supabase/server";

export async function GET(request: NextRequest) {
  try {
    const owner = await requireOwnerApi();
    const query = (request.nextUrl.searchParams.get("q") ?? "").trim();
    if (query.length < 2) return privateJson({ results: [] });
    if (isDemoMode()) {
      const terms = query.toLowerCase().split(/\s+/).filter(Boolean);
      const results = demoGraph.nodes.filter((node) => terms.some((term) => `${node.title} ${node.summary} ${node.body} ${node.scope_refs.join(" ")}`.toLowerCase().includes(term))).slice(0, 20);
      return privateJson({ results: results.map(({ id, type, status, title, summary, updated_at }) => ({ id, type, status, title, summary, updated_at })) });
    }
    const supabase = await createClient();
    const { data, error } = await supabase.from("nodes")
      .select("id,type,status,title,summary,updated_at")
      .eq("workspace_id", owner.workspaceId)
      .textSearch("search_vector", query, { type: "websearch", config: "english" })
      .order("updated_at", { ascending: false }).limit(20);
    if (error) throw error;
    return privateJson({ results: data ?? [] });
  } catch (error) {
    return apiError(error);
  }
}
