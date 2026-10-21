export type SetupClient = "codex" | "claude";
export interface SetupProject { ref: string; name: string }

function shellValue(value: string) {
  return `'${value.replaceAll("'", "'\\''")}'`;
}

export function agentSetup(endpoint: string, projectRef: string, client: SetupClient) {
  const url = new URL(endpoint);
  const local = ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname);
  if ((url.protocol !== "https:" && !(local && url.protocol === "http:")) || url.username || url.password || url.hash) {
    throw new Error("An HTTPS MCP endpoint is required.");
  }
  if (!/^project:[a-z0-9][a-z0-9._/-]*$/.test(projectRef)) throw new Error("Select a project first.");
  const server = local ? "loomtracer-local" : "loomtracer";
  const commands = client === "codex"
    ? `codex mcp add ${server} --url ${shellValue(endpoint)}\ncodex mcp login ${server} --scopes openid`
    : `claude mcp add --transport http ${server} --scope user ${shellValue(endpoint)}`;
  const instructions = `<!-- BEGIN LOOMTRACER PROJECT WORKFLOW -->
## Loomtracer

Use the hosted ${server} MCP connection (${endpoint}) for this repository's reasoning history. Its owning project is ${projectRef}. Confirm that reference with list_projects; never select another project as a fallback.

1. At the start of consequential work, call get_reasoning_brief with project_ref: "${projectRef}" and a concise task. Supply workflow_ref, target_ref, and context_refs when established by the task. Keep the fingerprint. Check complete, coverage_complete, scope_questions, and warnings; resolve missing scope and incomplete coverage before relying on guidance.
2. Compare your plan with the applicable server-authorized commitments in the brief. Use list_reasoning_datasets, search_reasoning, and expand_reasoning to inspect reasons, evidence, alternatives, and original sources. Follow search pagination. Only approved ancestor guidance returned by the server inherits; extraction interpretations and proposed contributions are not commitments. Current user instructions take precedence; surface material conflicts without silently changing authority. There is no separate plan-checking tool.
3. Before reporting completion, refresh get_reasoning_brief with the same task and scope plus previous_fingerprint. Reassess changed guidance, then compare the actual outcome and test evidence with applicable commitments. For reconstruction, get_reconstruction_packet accepts dataset_id, optional run_id, and source_cutoff; cite record IDs and keep unavailable reasons as not established.
4. Deduplicate relevant records before using capture_reasoning for durable, source-backed contributions and connections. Pass project_ref: "${projectRef}", dataset_id, a delta containing only new records and run metadata, expected_head, and a stable operation_id. The server derives parent revision and source coverage. Use a validated ReasoningBundle for a first import. Read back added records with expand_reasoning. Reuse the operation ID and exact payload after an uncertain response; refresh before resolving a stale head. Capture at most five durable entries per task, plus their explanatory connections; usually fewer. Skip routine activity, secrets, raw transcripts, and unsupported claims.
5. Use revise_extraction for extraction corrections: append a child run with replacements, omissions, review resolutions, and all prior record versions preserved. A changed human or agent position is a new contribution; an extraction correction does not change the original discussion.

Standing capture authorization is unratified. Neither capture_reasoning, revise_extraction, inferred adoption links, imported receipts, nor accepted review notes authorize commitments. Use change_commitment only for an explicitly authorized adopt, revise, or withdraw action and only with an active human-owner grant to this named agent for this exact project. Include rationale, expected_head, operation_id, and the exact prior_receipt_id for revision or withdrawal. The grant does not include descendant projects, delegation, or account administration. A granted agent can append a revision to a human decision; the original decision is never modified or deleted. Tool availability alone does not authorize an action.

Never treat an unavailable or incomplete brief as an empty graph. Report the specific connection problem once and continue independent safe work without substituting local SQLite, another account, or a retired connection. Do not read or print credentials to troubleshoot.
<!-- END LOOMTRACER PROJECT WORKFLOW -->`;
  const verificationPrompt = `Use the ${server} MCP connection at ${endpoint}. Refresh tool discovery and confirm the native reasoning tools are available. Call list_projects and confirm ${projectRef}. Then call get_reasoning_brief with project_ref: "${projectRef}" and task: "Verify the connection and review this project's guidance before starting work". Report the selected project, complete, coverage_complete, fingerprint, scope_questions, and warnings. Do not create records or change commitments for this connection check. If the project is missing or the brief is incomplete, report the problem without choosing another project. A successful discovery or brief does not grant commitment authority.`;
  return { server, commands, instructions, verificationPrompt, claudeImport: "@AGENTS.md" };
}
