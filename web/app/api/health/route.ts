import { HiveMindError, hivemind } from "@/lib/hivemind/client";
import { privateJson } from "@/lib/http";

export const dynamic = "force-dynamic";

/** 200 when the HiveMind server answers for this workspace's key; 503, naming why, when it does not. */
export async function GET() {
  try {
    const who = await hivemind().whoami();
    return privateJson({ ok: true, tenant: who.tenant_id });
  } catch (error) {
    return privateJson({ ok: false, reason: error instanceof HiveMindError ? error.code : "server" }, { status: 503 });
  }
}
