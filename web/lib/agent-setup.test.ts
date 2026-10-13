import { describe, expect, it } from "vitest";
import { agentSetup } from "./agent-setup";

describe("hosted agent setup", () => {
  it("uses OAuth and preserves the selected project throughout the workflow", () => {
    const setup = agentSetup("https://loomtracer.vercel.app/api/mcp", "project:team/product", "codex");
    expect(setup.commands).toContain("codex mcp login loomtracer --scopes openid");
    expect(setup.instructions).toContain('project_ref: "project:team/product"');
    expect(setup.instructions).toContain("previous_fingerprint");
    expect(setup.server).toBe("loomtracer");
    expect(setup.instructions).toContain("get_reasoning_brief");
    expect(setup.instructions).toContain("workflow_ref, target_ref, and context_refs");
    expect(setup.instructions).toContain("capture_reasoning");
    expect(setup.instructions).toContain("revise_extraction");
    expect(setup.instructions).toContain("get_reconstruction_packet");
    expect(setup.instructions).toContain("Standing capture authorization is unratified.");
    expect(setup.instructions).toContain("active human-owner grant to this named agent for this exact project");
    expect(setup.instructions).toContain("original decision is never modified or deleted");
    expect(setup.instructions).not.toMatch(/get_task_brief|check_plan_against_decisions|record_attempt|propose_decision|propose_assumption|propose_superseding_decision/);
    expect(setup.verificationPrompt).toContain("coverage_complete");
    expect(setup.verificationPrompt).toContain("does not grant commitment authority");
    expect(setup.verificationPrompt).toContain("Do not create records");
    expect(setup.claudeImport).toBe("@AGENTS.md");
  });

  it("provides a Claude user-scoped HTTP connection and a distinct local name", () => {
    const setup = agentSetup("http://127.0.0.1:3108/api/mcp", "project:demo", "claude");
    expect(setup.commands).toBe("claude mcp add --transport http loomtracer-local --scope user 'http://127.0.0.1:3108/api/mcp'");
    expect(setup.commands).not.toContain("Bearer");
  });

  it("rejects invalid project routing and insecure hosted endpoints", () => {
    expect(() => agentSetup("https://example.com/api/mcp", "all", "codex")).toThrow("Select a project");
    expect(() => agentSetup("http://example.com/api/mcp", "project:demo", "codex")).toThrow("HTTPS");
    expect(() => agentSetup("https://user:secret@example.com/api/mcp", "project:demo", "codex")).toThrow("HTTPS");
  });

  it("quotes copied endpoint text as a literal shell argument", () => {
    const setup = agentSetup("https://example.com/api/mcp?label='$(echo bad)'", "project:demo", "codex");
    expect(setup.commands).toContain("'https://example.com/api/mcp?label='\\''$(echo bad)'\\''' ".trim());
  });
});
