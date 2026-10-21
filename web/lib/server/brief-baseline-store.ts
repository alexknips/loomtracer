import type { SupabaseClient } from "@supabase/supabase-js";
import type { BriefBaselineStore, BriefBaselineManifest } from "./brief-baseline";

/** RPCs bind every token to the authenticated grant; the client never chooses a workspace. */
export function durableBriefBaselineStore(database: SupabaseClient): BriefBaselineStore {
  return {
    async get(token: string, projectRef: string, scopeKey: string) {
      const { data, error } = await database.rpc("get_agent_brief_baseline", {
        baseline_token: token, target_project_ref: projectRef, scope_key: scopeKey,
      });
      if (error) throw error;
      return data ?? null;
    },
    async save(projectRef: string, scopeKey: string, manifest: BriefBaselineManifest) {
      const { data, error } = await database.rpc("save_agent_brief_baseline", {
        target_project_ref: projectRef, scope_key: scopeKey, baseline_manifest: manifest,
      });
      if (error) throw error;
      if (!data?.token || !data.expires_at) throw new Error("No durable baseline was issued.");
      return { token: data.token, expires_at: data.expires_at };
    },
  };
}
