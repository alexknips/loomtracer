import { WebStandardStreamableHTTPServerTransport } from "@modelcontextprotocol/sdk/server/webStandardStreamableHttp.js";
import { buildMcpServer } from "@/lib/server/mcp";
import { PRIVATE_HEADERS, privateJson } from "@/lib/server/http";
import { authenticatePat } from "@/lib/server/tokens";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

function methodNotAllowed() {
  return privateJson({ jsonrpc: "2.0", error: { code: -32000, message: "Method not allowed." }, id: null }, { status: 405, headers: { Allow: "POST" } });
}

export async function GET() { return methodNotAllowed(); }
export async function DELETE() { return methodNotAllowed(); }

export async function POST(request: Request) {
  const pat = await authenticatePat(request);
  if (!pat) return privateJson({ jsonrpc: "2.0", error: { code: -32001, message: "Unauthorized" }, id: null }, { status: 401, headers: { "WWW-Authenticate": "Bearer" } });
  const server = buildMcpServer(pat);
  const transport = new WebStandardStreamableHTTPServerTransport({ sessionIdGenerator: undefined, enableJsonResponse: true });
  try {
    await server.connect(transport);
    const response = await transport.handleRequest(request, {
      authInfo: { token: pat.tokenId, clientId: pat.actor.id, scopes: pat.scopes },
    });
    const headers = new Headers(response.headers);
    for (const [key, value] of Object.entries(PRIVATE_HEADERS)) headers.set(key, value);
    return new Response(response.body, { status: response.status, statusText: response.statusText, headers });
  } catch {
    return privateJson({ jsonrpc: "2.0", error: { code: -32603, message: "Internal server error" }, id: null }, { status: 500 });
  } finally {
    await transport.close().catch(() => undefined);
    await server.close().catch(() => undefined);
  }
}
