import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ context: { user: { id: "owner", email: "owner@example.test" }, workspaceId: "team", workspaceName: "Team", membershipRole: "owner", isAdmin: false, demo: false }, rpc: vi.fn() }));
vi.mock("@/lib/server/auth", () => ({ requireOwnerApi: vi.fn(async () => mocks.context) }));
vi.mock("@/lib/supabase/server", () => ({ createClient: vi.fn(async () => ({ rpc: mocks.rpc })) }));
vi.mock("@/lib/env", () => ({ isDemoMode: () => false, appOrigin: () => "http://localhost:3000" }));
import { POST as invite } from "@/app/api/team/invitations/route";
import { DELETE as remove } from "@/app/api/team/members/[id]/route";
import { PATCH as reviewRequest } from "@/app/api/admin/access-requests/[id]/route";
import { POST as ratify } from "@/app/api/review/[id]/ratify/route";

const uuid = "11111111-1111-4111-8111-111111111111";
const params = { params: Promise.resolve({ id: uuid }) };
const request = (path: string, method: string, body?: unknown, origin = "http://localhost:3000") => new Request(`http://localhost:3000${path}`, { method, headers: { origin, "Content-Type": "application/json" }, body: body ? JSON.stringify(body) : undefined });
beforeEach(() => {
  Object.assign(mocks.context, { membershipRole: "owner", isAdmin: false, demo: false });
  mocks.rpc.mockReset();
  mocks.rpc.mockResolvedValue({ data: { id: uuid, email: "member@example.test", secret: "a".repeat(64) }, error: null });
});
describe("team boundary at HTTP endpoints", () => {
  it("denies member administration even when the member is a platform admin", async () => {
    Object.assign(mocks.context, { membershipRole: "reviewer", isAdmin: true });
    expect((await invite(request("/api/team/invitations", "POST", { email: "member@example.test" }))).status).toBe(403);
    expect((await remove(request(`/api/team/members/${uuid}`, "DELETE"), params)).status).toBe(403);
    expect(mocks.rpc).not.toHaveBeenCalled();
  });
  it("derives the invitation destination on the server and returns a fragment link", async () => {
    const response = await invite(request("/api/team/invitations", "POST", { email: "member@example.test", workspaceId: "foreign" }));
    expect(response.status).toBe(200);
    expect(mocks.rpc).toHaveBeenCalledWith("create_workspace_invitation", { invited_email: "member@example.test" });
    expect((await response.json()).link).toBe(`http://localhost:3000/join#${uuid}.${"a".repeat(64)}`);
    expect(response.headers.get("cache-control")).toContain("no-store");
  });
  it("blocks cross-origin mutation before invoking the database", async () => {
    const response = await invite(request("/api/team/invitations", "POST", { email: "member@example.test" }, "https://attacker.example"));
    expect(response.status).toBe(403); expect(mocks.rpc).not.toHaveBeenCalled();
  });
  it("does not give ordinary owners access-request administration", async () => {
    expect((await reviewRequest(request(`/api/admin/access-requests/${uuid}`, "PATCH", { action: "approve" }), params)).status).toBe(403);
    expect(mocks.rpc).not.toHaveBeenCalled();
  });
  it("requires the displayed record version for human review", async () => {
    const response = await ratify(request(`/api/review/${uuid}/ratify`, "POST", { reconsider_conditions: ["Requirements change"] }), params);
    expect(response.status).toBe(400); expect(mocks.rpc).not.toHaveBeenCalled();
  });
  it("returns a refresh-required conflict for a changed record and forwards the version", async () => {
    mocks.context.membershipRole = "reviewer";
    mocks.rpc.mockResolvedValue({ data: null, error: { message: "STALE_REVIEW" } });
    const expected_updated_at = "2026-09-30T00:00:00.000Z";
    const response = await ratify(request(`/api/review/${uuid}/ratify`, "POST", { expected_updated_at, reconsider_conditions: ["Requirements change"] }), params);
    expect(response.status).toBe(409);
    expect((await response.json()).code).toBe("STALE_REVIEW");
    expect(mocks.rpc).toHaveBeenCalledWith("ratify_node", expect.objectContaining({ expected_updated_at, target_workspace_id: "team" }));
  });
});
