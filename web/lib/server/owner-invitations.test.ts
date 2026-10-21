import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  context: { user: { id: "admin" }, workspaceId: "original", membershipRole: "owner", isAdmin: true, demo: false },
  rpc: vi.fn(),
}));
vi.mock("@/lib/server/auth", () => ({ requireOwnerApi: vi.fn(async () => mocks.context) }));
vi.mock("@/lib/supabase/server", () => ({ createClient: vi.fn(async () => ({ rpc: mocks.rpc })) }));
vi.mock("@/lib/env", () => ({ appOrigin: () => "http://localhost:3000" }));
import { POST as invite, GET as list } from "@/app/api/invitations/route";
import { DELETE as revoke } from "@/app/api/invitations/[id]/route";
import { PATCH as review } from "@/app/api/admin/access-requests/[id]/route";
import { PATCH as changeStatus } from "@/app/api/accounts/[id]/route";

const id = "11111111-1111-4111-8111-111111111111";
const params = { params: Promise.resolve({ id }) };
const request = (body: unknown, origin = "http://localhost:3000") => new Request("http://localhost:3000/api/invitations", {
  method: "POST", headers: { origin, "Content-Type": "application/json" }, body: JSON.stringify(body),
});
beforeEach(() => {
  Object.assign(mocks.context, { membershipRole: "owner", isAdmin: true, demo: false });
  mocks.rpc.mockReset();
  mocks.rpc.mockResolvedValue({ data: { id, email: "owner@example.test", workspace_name: "Studio", kind: "owner", secret: "a".repeat(64), expires_at: "2026-10-08T00:00:00Z" }, error: null });
});

describe("owner invitations", () => {
  it("normalizes names and emails and ignores client-selected workspace and role", async () => {
    const response = await invite(request({ email: " OWNER@example.test ", workspaceName: " Studio ", workspaceId: "original", kind: "member", role: "admin" }));
    expect(response.status).toBe(200);
    expect(mocks.rpc).toHaveBeenCalledWith("create_beta_invitation", { invited_email: "owner@example.test", invited_workspace_name: "Studio" });
    const data = await response.json();
    expect(data).toMatchObject({ workspace_name: "Studio", kind: "owner", link: `http://localhost:3000/join#${id}.${"a".repeat(64)}` });
    expect(data).not.toHaveProperty("secret");
    expect(response.headers.get("cache-control")).toContain("no-store");
  });
  it.each([undefined, "", "   ", "a".repeat(81)])("rejects an absent or invalid workspace name (%s)", async workspaceName => {
    expect((await invite(request({ email: "owner@example.test", workspaceName }))).status).toBe(400);
    expect(mocks.rpc).not.toHaveBeenCalled();
  });
  it("denies owner-admission administration to ordinary workspace owners and reviewers", async () => {
    mocks.context.isAdmin = false;
    for (const membershipRole of ["owner", "reviewer"]) {
      mocks.context.membershipRole = membershipRole;
      expect((await invite(request({ email: "owner@example.test", workspaceName: "Studio" }))).status).toBe(403);
      expect((await list()).status).toBe(403);
      expect((await revoke(request({}), params)).status).toBe(403);
    }
    expect(mocks.rpc).not.toHaveBeenCalled();
  });
  it("keeps demo administration read-only", async () => {
    mocks.context.demo = true;
    expect((await invite(request({ email: "owner@example.test", workspaceName: "Studio" }))).status).toBe(403);
    expect((await revoke(request({}), params)).status).toBe(403);
    expect((await changeStatus(request({ status: "suspended" }), params)).status).toBe(403);
    expect(await (await list()).json()).toEqual({ enabled: false, invitations: [], accounts: [] });
    expect(mocks.rpc).not.toHaveBeenCalled();
  });
  it("rejects cross-origin requests before touching the database", async () => {
    expect((await invite(request({ email: "owner@example.test", workspaceName: "Studio" }, "https://elsewhere.test"))).status).toBe(403);
    expect(mocks.rpc).not.toHaveBeenCalled();
  });
  it.each(["this identity already belongs to a workspace", "an invitation is already pending"])("reports an actionable invitation conflict: %s", async message => {
    mocks.rpc.mockResolvedValue({ data: null, error: { message } });
    const response = await invite(request({ email: "owner@example.test", workspaceName: "Studio" }));
    expect(response.status).toBe(409);
    expect((await response.json()).error).toContain("workspace");
  });
  it("does not return internal database details", async () => {
    mocks.rpc.mockResolvedValue({ data: null, error: { message: "internal relation secret_hash error" } });
    const response = await invite(request({ email: "owner@example.test", workspaceName: "Studio" }));
    expect(response.status).toBe(400);
    expect(await response.text()).not.toContain("secret_hash");
  });
  it("requires a named workspace for request approval and leaves dismissal independent", async () => {
    expect((await review(request({ action: "approve" }), params)).status).toBe(400);
    expect(mocks.rpc).not.toHaveBeenCalled();
    expect((await review(request({ action: "approve", workspaceName: " Studio " }), params)).status).toBe(200);
    expect(mocks.rpc).toHaveBeenLastCalledWith("approve_access_request", { request_id: id, invited_workspace_name: "Studio" });
    expect((await review(request({ action: "dismiss" }), params)).status).toBe(200);
    expect(mocks.rpc).toHaveBeenLastCalledWith("dismiss_access_request", { request_id: id });
  });
});
