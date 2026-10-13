import { createClient } from "@supabase/supabase-js";
import { isDemoMode, publicSupabaseEnv } from "@/lib/env";
import { privateJson } from "@/lib/server/http";
export const dynamic = "force-dynamic";
export async function GET() {
  if (isDemoMode()) return privateJson({ ok: true, mode: "demo" });
  try {
    const env = publicSupabaseEnv();
    const client = createClient(env.url, env.publishableKey, { auth: { persistSession: false } });
    const [access, reasoning] = await Promise.all([client.rpc("beta_health"), client.rpc("reasoning_health")]);
    const ready = !access.error && access.data === true && !reasoning.error && reasoning.data?.ready === true;
    return privateJson({ ok: ready, architecture: "entry-connection", schemaVersion: "0.3.0",
      mode: reasoning.error ? "unavailable" : reasoning.data?.mode ?? "unavailable" }, { status: ready ? 200 : 503 });
  } catch { return privateJson({ ok: false }, { status: 503 }); }
}
