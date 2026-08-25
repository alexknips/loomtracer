import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod/v4";
import { HostedGraph, type CaptureInput } from "@/lib/server/hosted-graph";
import type { PatContext } from "@/lib/server/tokens";

const stringList = z.array(z.string()).default([]);
const level = z.enum(["low", "medium", "high"]).optional();
const common = {
  title: z.string().min(1), summary: z.string().default(""), body: z.string().default(""),
  scope_type: z.string().nullable().optional(), scope_refs: stringList,
  actor: z.string().optional().describe("Ignored. Agent identity is derived from the bearer token."),
  evidence_summary: z.string().default(""), source_ref: z.string().default(""),
};
const decision = {
  ...common, confidence: level, reversibility: level, consequence: level,
  chosen_option: z.string().nullable().optional(), rejected_options: stringList,
  reconsider_conditions: stringList,
};

function complete(text: string, structuredContent: Record<string, unknown>) {
  return { content: [{ type: "text" as const, text }], structuredContent };
}

function input(args: Record<string, unknown>): CaptureInput {
  const trusted = { ...args };
  delete trusted.actor;
  return trusted as unknown as CaptureInput;
}

export function buildMcpServer(pat: PatContext) {
  const graph = new HostedGraph(pat);
  const server = new McpServer({ name: "neurofilament", version: "0.2.0" }, {
    instructions: "Use active decisions as advisory constraints. Candidate entries are unratified. Agents may propose entries but cannot ratify or reject them.",
  });

  server.registerTool("get_relevant_context", {
    title: "Get Relevant Neurofilament Context",
    description: "Retrieve compact prior decisions and attempts before planning repo work.",
    inputSchema: { task: z.string().min(1), files: stringList, technologies: stringList, proposed_plan: z.string().default(""), token_budget: z.number().int().min(100).max(4000).default(700) },
  }, async ({ task, files, technologies, proposed_plan, token_budget }) => {
    const result = await graph.relevantContext(task, files, technologies, proposed_plan, token_budget);
    return complete(result.text, { context: result.text, matches: result.matches.map((match) => ({ node: match.toolNode, score: match.score })) });
  });

  server.registerTool("expand_node", {
    title: "Expand Neurofilament Node", description: "Return full details and connected edges for a graph node.",
    inputSchema: { id: z.string().min(1) },
  }, async ({ id }) => { const result = await graph.expand(id); return complete(result.text, result.structured); });

  server.registerTool("record_attempt", {
    title: "Record Attempt", description: "Record an unratified attempt or failed path.",
    inputSchema: { ...common, outcome: z.string().default("") },
  }, async (args) => {
    try { const result = await graph.capture("Attempt", input(args)); return complete(`Created unratified attempt: ${result.node.id}`, { node: result.node }); }
    catch (error) { return { content: [{ type: "text" as const, text: error instanceof Error ? error.message : "Capture failed." }], isError: true }; }
  });

  server.registerTool("propose_decision", {
    title: "Propose Decision", description: "Create a candidate Decision for later human ratification.", inputSchema: decision,
  }, async (args) => {
    try { const result = await graph.capture("Decision", input(args)); return complete(`Created candidate decision: ${result.node.id}. A human must ratify it before it becomes active.`, { node: result.node }); }
    catch (error) { return { content: [{ type: "text" as const, text: error instanceof Error ? error.message : "Capture failed." }], isError: true }; }
  });

  server.registerTool("propose_assumption", {
    title: "Propose Assumption", description: "Create a proposed Assumption for later human acceptance.",
    inputSchema: { ...common, reconsider_conditions: stringList },
  }, async (args) => {
    try { const result = await graph.capture("Assumption", input(args)); return complete(`Created proposed assumption: ${result.node.id}. A human must accept it before it becomes durable guidance.`, { node: result.node }); }
    catch (error) { return { content: [{ type: "text" as const, text: error instanceof Error ? error.message : "Capture failed." }], isError: true }; }
  });

  server.registerTool("check_plan_against_decisions", {
    title: "Check Plan Against Decisions", description: "Warn when a plan contradicts active Decisions. Advisory only.",
    inputSchema: { task: z.string().min(1), plan: z.string().min(1) },
  }, async ({ task, plan }) => { const result = await graph.checkPlan(task, plan); return complete(result.text, result.structured); });

  server.registerTool("propose_superseding_decision", {
    title: "Propose Superseding Decision", description: "Create a candidate that proposes to supersede an active Decision.",
    inputSchema: { ...decision, supersedes_id: z.string().min(1), reason: z.string().min(1) },
  }, async ({ supersedes_id, reason, ...args }) => {
    try {
      const result = await graph.capture("Decision", input(args), { id: supersedes_id, reason });
      return complete(`Created candidate superseding decision: ${result.node.id}. It proposes to supersede ${supersedes_id} via ${result.proposedEdgeId}, but a human must ratify it.`, { node: result.node, proposed_edge_id: result.proposedEdgeId });
    } catch (error) { return { content: [{ type: "text" as const, text: error instanceof Error ? error.message : "Capture failed." }], isError: true }; }
  });

  return server;
}
