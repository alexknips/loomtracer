import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import { NextRequest, NextResponse } from "next/server";
const refresh = vi.hoisted(() => vi.fn(async () => NextResponse.next()));
vi.mock("@/lib/supabase/proxy", () => ({ refreshSupabaseSession: refresh }));
import { proxy } from "../proxy";
beforeEach(() => refresh.mockClear());
afterEach(() => vi.unstubAllEnvs());
describe("public route boundary", () => {
  it.each(["/", "/demo", "/api/access-requests", "/og.png"])("never refreshes a private session on %s", async path => {
    const response = await proxy(new NextRequest(`https://demo.example${path}`));
    expect(response.status).toBe(200); expect(refresh).not.toHaveBeenCalled();
  });
  it.each(["/graph", "/login", "/api/mcp", "/api/session", "/oauth/consent", "/auth/callback", "/.well-known/oauth-protected-resource/api/mcp", "/api/admin/access-requests"])("public-only deployment blocks %s before auth", async path => {
    vi.stubEnv("NEUROFILAMENT_PUBLIC_ONLY", "true");
    const response = await proxy(new NextRequest(`https://demo.example${path}`));
    expect(response.status).toBe(404); expect(refresh).not.toHaveBeenCalled();
  });
  it("public-only allows intake POST, never private POST or server actions", async () => {
    vi.stubEnv("NEUROFILAMENT_PUBLIC_ONLY", "true");
    expect((await proxy(new NextRequest("https://demo.example/api/access-requests", { method: "POST" }))).status).toBe(200);
    expect((await proxy(new NextRequest("https://demo.example/api/team/invitations", { method: "POST" }))).status).toBe(404);
    expect((await proxy(new NextRequest("https://demo.example/", { method: "POST" }))).status).toBe(404);
    expect(refresh).not.toHaveBeenCalled();
  });
  it("full app still refreshes authenticated routes", async () => {
    await proxy(new NextRequest("https://app.example/graph"));
    expect(refresh).toHaveBeenCalledOnce();
  });
});
