import { WebStandardStreamableHTTPServerTransport } from "@modelcontextprotocol/sdk/server/webStandardStreamableHttp.js";
import { buildMcpServer } from "@/lib/server/mcp";
import { PRIVATE_HEADERS, privateJson } from "@/lib/server/http";
import { authenticateAgent, mcpResource } from "@/lib/server/agent-auth";
import { readReasoningJson } from "@/lib/server/reasoning-http";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

function methodNotAllowed() {
  return privateJson({ jsonrpc: "2.0", error: { code: -32000, message: "Method not allowed." }, id: null }, { status: 405, headers: { Allow: "POST" } });
}

export async function GET() { return methodNotAllowed(); }
export async function DELETE() { return methodNotAllowed(); }

export async function POST(request: Request) {
  const agent = await authenticateAgent(request);
  if (!agent) return privateJson({ jsonrpc: "2.0", error: { code: -32001, message: "Unauthorized" }, id: null }, { status: 401, headers: { "WWW-Authenticate": `Bearer resource_metadata="${new URL("/.well-known/oauth-protected-resource/api/mcp", mcpResource()).href}"` } });
  let boundedRequest: Request;
  try {
    const body = await readReasoningJson(request);
    boundedRequest = new Request(request.url, { method: "POST", headers: request.headers, body: JSON.stringify(body) });
  } catch (error) {
    const tooLarge = error instanceof Error && error.message === "BUNDLE_TOO_LARGE";
    return privateJson({ jsonrpc: "2.0", error: { code: -32600, message: tooLarge ? "Request exceeds 4 MB. Use an incremental delta." : "Invalid JSON request." }, id: null }, { status: tooLarge ? 413 : 400 });
  }
  const server = buildMcpServer(agent);
  const transport = new WebStandardStreamableHTTPServerTransport({ sessionIdGenerator: undefined, enableJsonResponse: true });
  try {
    await server.connect(transport);
    const response = await transport.handleRequest(boundedRequest, {
      authInfo: { token: agent.connectionId, clientId: agent.clientId, scopes: agent.scopes },
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
