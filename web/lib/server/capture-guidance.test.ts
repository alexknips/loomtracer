import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
const mock = vi.hoisted(() => ({ save: vi.fn() }));
vi.mock("./reasoning-context", () => ({ agentReasoning: () => mock }));
import { protocol, protocolBundle, resultValue } from "./reasoning-test-support";
beforeEach(() => vi.clearAllMocks()); afterEach(() => vi.unstubAllEnvs());
describe("native capture guidance", () => {
  it("explains immutable history, qualified rationale and separate commitment authority", async () => {
    await protocol(async client => {
      for (const phrase of ["immutable Entries and Connections", "do not invent missing reasons", "do not authorize commitments", "human commitment", "Imported receipts, inferred links, and review notes never establish authority"]) expect(client.getInstructions()).toContain(phrase);
    });
  });
  it("does not revive retired schemas when legacy pilot flags change", async () => {
    const initial = await protocol(async client => ({ instructions: client.getInstructions(), tools: await client.listTools() }));
    vi.stubEnv("NEUROFILAMENT_PILOT_ENABLED", "1"); vi.stubEnv("NEUROFILAMENT_CAPTURE_GUIDANCE", "off");
    expect(await protocol(async client => ({ instructions: client.getInstructions(), tools: await client.listTools() }))).toEqual(initial);
  });
  it.each(["capture_reasoning", "revise_extraction"])("passes exact qualified sources and grouped connections through %s", async name => {
    const bundle = protocolBundle();
    const qualified = "For bulk-import only: retain 28k records for 167h. 4x is observed, not approved policy.\n" + "Necessary qualifications. ".repeat(250) + "Exception: legal holds are indefinite. Contrary evidence remains unresolved.";
    const original = bundle.sources[0].content; bundle.sources[0].content += "\n" + qualified;
    mock.save.mockResolvedValue({ bundle, head: "saved-head", commitments: [] });
    const result = await protocol(client => client.callTool({ name, arguments: { project_ref: "project:test", bundle, expected_head: "prior-head", operation_id: "capture-qualified" } }));
    expect(result.isError).not.toBe(true); expect(mock.save).toHaveBeenCalledWith("project:test", bundle, "prior-head", "capture-qualified");
    const received = mock.save.mock.calls[0][1]; expect(received.sources[0].content).toBe(original + "\n" + qualified);
    expect(received.connections.find((connection: { id: string }) => connection.id === "C-support:v1").participants).toHaveLength(4);
    expect(resultValue(result)).toMatchObject({ authorityChanged: false, head: "saved-head" });
  });
});
