import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({ context: vi.fn(), save: vi.fn(), read: vi.fn(), commit: vi.fn(), grant: vi.fn(), revoke: vi.fn() }));
vi.mock("./reasoning-context", () => ({ humanReasoning: mocks.context }));
import { GET, POST } from "@/app/api/reasoning/route";
import { POST as commit } from "@/app/api/reasoning/commitments/route";
import { POST as grant, DELETE as revoke } from "@/app/api/reasoning/grants/route";
import { readReasoningJson, HOSTED_BUNDLE_LIMIT } from "./reasoning-http";

const origin = "https://loomtracer.vercel.app";
const request = (body: unknown, path = "/api/reasoning", suppliedOrigin = origin) => new Request(origin + path, {
  method: "POST", headers: { "Content-Type": "application/json", Origin: suppliedOrigin }, body: JSON.stringify(body),
});
beforeEach(() => {
  vi.stubEnv("NODE_ENV", "production"); vi.stubEnv("APP_URL", origin);
  vi.clearAllMocks();
  mocks.context.mockResolvedValue({ save: mocks.save, read: mocks.read, commit: mocks.commit, grant: mocks.grant, revoke: mocks.revoke });
  mocks.save.mockResolvedValue({ head: "new", bundle: { id: "test" } });
});
afterEach(() => vi.unstubAllEnvs());
describe("reasoning HTTP boundaries", () => {
  const capture = { project_ref: "project:test", bundle: { id: "test" }, expected_head: null, operation_id: "retry-0001" };
  it("requires an admitted authenticated workspace for reads and writes", async () => {
    mocks.context.mockRejectedValue(new Error("UNAUTHORIZED"));
    expect((await GET(new Request(origin + "/api/reasoning"))).status).toBe(401);
    expect((await POST(request(capture))).status).toBe(401);
    expect(mocks.save).not.toHaveBeenCalled();
  });
  it("rejects cross-origin writes before resolving private state", async () => {
    expect((await POST(request(capture, undefined, "https://unrelated.example"))).status).toBe(403);
    expect(mocks.context).not.toHaveBeenCalled();
  });
  it("preserves the exact request, expected head and stable retry ID", async () => {
    const response = await POST(request(capture));
    expect(response.status).toBe(200);
    expect(response.headers.get("cache-control")).toContain("no-store");
    expect(mocks.save).toHaveBeenCalledWith("project:test", capture.bundle, null, "retry-0001");
  });
  it("does not accept client actor or authority overrides alongside capture", async () => {
    expect((await POST(request({ ...capture, recorded_by: "human:owner" }))).status).toBe(400);
    expect(mocks.save).not.toHaveBeenCalled();
  });
  it("reports stale operations and maintenance without exposing database errors", async () => {
    mocks.save.mockRejectedValueOnce(new Error("STALE_HEAD private-detail"));
    const stale = await POST(request(capture)); expect(stale.status).toBe(409);
    expect(await stale.text()).not.toContain("private-detail");
    mocks.save.mockRejectedValueOnce(new Error("MAINTENANCE private-detail"));
    const frozen = await POST(request(capture)); expect(frozen.status).toBe(503);
    expect(frozen.headers.get("retry-after")).toBe("60");
  });
  it("commitment action passes exact prior receipt and requires rationale", async () => {
    const action = { project_ref: "project:test", dataset_id: "test", action: "revise", entry_id: "E2", prior_receipt_id: "receipt-1", rationale: "New evidence", expected_head: "head-1", operation_id: "commit-0001" };
    mocks.commit.mockResolvedValue({ head: "head-2" });
    expect((await commit(request(action))).status).toBe(200);
    expect(mocks.commit).toHaveBeenCalledWith("project:test", "test", "revise", "E2", "receipt-1", "New evidence", "head-1", "commit-0001");
    expect((await commit(request({ ...action, rationale: "" }))).status).toBe(400);
  });
  it("does not convert member or agent capture rights into grant rights", async () => {
    mocks.grant.mockRejectedValue(new Error("FORBIDDEN")); mocks.revoke.mockRejectedValue(new Error("FORBIDDEN"));
    const id = "11111111-1111-4111-8111-111111111111";
    expect((await grant(request({ project_ref: "project:test", connection_id: id }))).status).toBe(403);
    expect((await revoke(request({ project_ref: "project:test", grant_id: id }))).status).toBe(403);
  });
  it("bounds streamed requests even if Content-Length is absent", async () => {
    await expect(readReasoningJson(request({ value: "x".repeat(HOSTED_BUNDLE_LIMIT) }))).rejects.toThrow("BUNDLE_TOO_LARGE");
  });
});
