import { isDemoMode } from "@/lib/env";
import { createAdminClient } from "@/lib/supabase/admin";
import { privateJson } from "@/lib/server/http";

export const dynamic = "force-dynamic";

export async function GET() {
  if (isDemoMode()) return privateJson({ ok: true, mode: "demo", database: "not-checked" });
  const required = ["NEXT_PUBLIC_SUPABASE_URL", "NEXT_PUBLIC_SUPABASE_PUBLISHABLE_KEY", "SUPABASE_SECRET_KEY", "OWNER_EMAIL", "APP_URL"];
  const missing = required.filter((name) => !process.env[name]?.trim());
  if (missing.length) return privateJson({ ok: false, configuration: "incomplete", missing }, { status: 503 });
  try {
    const admin = createAdminClient();
    const { error } = await admin.from("workspaces").select("id", { head: true, count: "exact" }).limit(1);
    if (error) throw error;
    return privateJson({ ok: true, configuration: "ready", database: "reachable" });
  } catch {
    return privateJson({ ok: false, configuration: "ready", database: "unreachable" }, { status: 503 });
  }
}
