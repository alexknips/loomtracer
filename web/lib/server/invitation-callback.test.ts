import { beforeEach, describe, expect, it, vi } from "vitest";
import { NextRequest } from "next/server";
import { PROJECT_COOKIE } from "@/lib/projects";

const mocks = vi.hoisted(() => ({ exchange: vi.fn(), user: vi.fn(), signOut: vi.fn(), rpc: vi.fn() }));
vi.mock("@supabase/ssr", () => ({ createServerClient: vi.fn(() => ({
  auth: { exchangeCodeForSession: mocks.exchange, getUser: mocks.user, signOut: mocks.signOut }, rpc: mocks.rpc,
})) }));
vi.mock("@/lib/env", () => ({ appOrigin: () => "http://localhost:3000", publicSupabaseEnv: () => ({ url: "http://localhost:55421", publishableKey: "fixture" }) }));
import { GET } from "@/app/auth/callback/route";

const invitation = "11111111-1111-4111-8111-111111111111." + "a".repeat(64);
const request = (query = "?code=fixture&next=/onboarding") => new NextRequest(`http://localhost:3000/auth/callback${query}`, {
  headers: { cookie: `nf_invitation=${invitation}; ${PROJECT_COOKIE}=project:old-workspace` },
});
beforeEach(() => {
  vi.clearAllMocks();
  mocks.exchange.mockResolvedValue({ error: null });
  mocks.user.mockResolvedValue({ data: { user: { id: "invitee" } }, error: null });
  mocks.signOut.mockResolvedValue({ error: null });
  mocks.rpc.mockResolvedValue({ data: [{ workspace_id: "new-workspace" }], error: null });
});
const expectCleared = (response: Awaited<ReturnType<typeof GET>>) => {
  expect(response.cookies.get("nf_invitation")?.value).toBe("");
  expect(response.cookies.get(PROJECT_COOKIE)?.value).toBe("");
  expect(response.headers.get("cache-control")).toContain("no-store");
};

describe("invitation callback handoff", () => {
  it("redeems the cookie on the exchanged session before resolving membership", async () => {
    const response = await GET(request());
    expect(mocks.rpc.mock.calls).toEqual([
      ["redeem_beta_invitation", { invitation_id: invitation.split(".")[0], invitation_secret: "a".repeat(64) }],
      ["get_workspace_context"],
    ]);
    expect(response.headers.get("location")).toBe("http://localhost:3000/onboarding");
    expectCleared(response);
  });
  it("signs out and clears staged state when redemption fails", async () => {
    mocks.rpc.mockResolvedValue({ data: null, error: { message: "invitation unavailable" } });
    const response = await GET(request());
    expect(response.headers.get("location")).toBe("http://localhost:3000/login?error=unauthorized");
    expect(mocks.signOut).toHaveBeenCalledOnce();
    expect(mocks.signOut).toHaveBeenCalledWith({ scope: "local" });
    expect(mocks.rpc).toHaveBeenCalledTimes(1);
    expectCleared(response);
  });
  it("clears a stale invitation after an expired PKCE exchange", async () => {
    mocks.exchange.mockResolvedValue({ error: { message: "expired code" } });
    const response = await GET(request());
    expect(response.headers.get("location")).toBe("http://localhost:3000/login?error=callback");
    expect(mocks.rpc).not.toHaveBeenCalled();
    expectCleared(response);
  });
  it("clears a stale invitation after a missing or cancelled callback code", async () => {
    const response = await GET(request("?error=access_denied"));
    expect(mocks.exchange).not.toHaveBeenCalled();
    expectCleared(response);
  });
});
