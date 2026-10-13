import { mcpResource } from "@/lib/server/agent-auth";
import { publicSupabaseEnv } from "@/lib/env";
export async function GET() {
  return Response.json({ resource: mcpResource(), authorization_servers: [`${publicSupabaseEnv().url}/auth/v1`],
    bearer_methods_supported: ["header"], scopes_supported: ["openid"], resource_name: "Loomtracer" });
}
