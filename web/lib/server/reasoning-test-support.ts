import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { InMemoryTransport } from "@modelcontextprotocol/sdk/inMemory.js";
import { upgradeBundle } from "@reasoning/domain";
import { makeDemoBundle } from "../../../prototypes/reasoning/src/fixture";
import type { AgentContext } from "./agent-auth";
import { buildMcpServer } from "./mcp";
export const protocolAgent: AgentContext = { userId: "user", clientId: "client", accessToken: "test", connectionId: "connection", workspaceId: "workspace", scopes: ["context:read", "entries:write"], actor: { id: "agent", kind: "agent", display_name: "Test agent", platform: "test" } };
export function protocolBundle() { const bundle = upgradeBundle(makeDemoBundle()); bundle.contexts[0].kind = "discussion"; return bundle; }
export async function protocol<T>(run: (client: Client) => Promise<T>, agent = protocolAgent) {
  const server = buildMcpServer(agent), client = new Client({ name: "native-reasoning-test", version: "1" });
  const [a, b] = InMemoryTransport.createLinkedPair();
  try { await server.connect(b); await client.connect(a); return await run(client); }
  finally { await client.close(); await server.close(); }
}
export function resultValue<T = Record<string, unknown>>(result: unknown): T {
  return (result as { structuredContent: { result: T } }).structuredContent.result;
}
