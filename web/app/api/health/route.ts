import { LoomtracerError, loomtracer } from "@/lib/loomtracer/client";
import { privateJson } from "@/lib/http";

export const dynamic = "force-dynamic";

/** 200 when the Loomtracer server answers for this workspace's key; 503, naming why, when it does not. */
export async function GET() {
  try {
    const who = await loomtracer().whoami();
    return privateJson({ ok: true, tenant: who.tenant_id });
  } catch (error) {
    return privateJson({ ok: false, reason: error instanceof LoomtracerError ? error.code : "server" }, { status: 503 });
  }
}
