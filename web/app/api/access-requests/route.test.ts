import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
const { rpc, client } = vi.hoisted(() => ({ rpc: vi.fn(), client: vi.fn() }));
vi.mock("@supabase/supabase-js", () => ({ createClient: client }));
import { POST } from "./route";

const endpoint = "https://public.example/api/access-requests";
const valid = { email: "builder@example.com", use_case: "Remember decisions across our coding agents.", website: "" };
function request(body: unknown = valid, headers: Record<string, string> = {}) {
  return new Request(endpoint, { method: "POST", headers: { Origin: "https://public.example", "Content-Type": "application/json", ...headers }, body: typeof body === "string" ? body : JSON.stringify(body) });
}
beforeEach(() => {
  vi.stubEnv("NODE_ENV", "production"); vi.stubEnv("APP_URL", "https://public.example");
  vi.stubEnv("NEXT_PUBLIC_SUPABASE_URL", "https://test.supabase.co");
  vi.stubEnv("NEXT_PUBLIC_SUPABASE_PUBLISHABLE_KEY", "sb_publishable_fixture");
  rpc.mockReset(); client.mockReset(); client.mockReturnValue({ rpc }); rpc.mockResolvedValue({ data: { ok: true }, error: null });
});
afterEach(() => vi.unstubAllEnvs());

describe("public access request boundary", () => {
  it("accepts only a confirmed intake result using an anonymous client", async () => {
    const response = await POST(request());
    expect(response.status).toBe(202); expect(await response.json()).toEqual({ ok: true });
    expect(rpc).toHaveBeenCalledWith("submit_access_request", { request_email: valid.email, request_use_case: valid.use_case, honeypot: "" });
    expect(client).toHaveBeenCalledWith("https://test.supabase.co", "sb_publishable_fixture", expect.objectContaining({ auth: { persistSession: false, autoRefreshToken: false, detectSessionInUrl: false } }));
    expect(response.headers.get("cache-control")).toContain("no-store");
  });
  it("rejects cross-origin, malformed, invalid and oversized input before database access", async () => {
    expect((await POST(request(valid, { Origin: "https://attacker.example" }))).status).toBe(403);
    expect((await POST(request(valid, { "Content-Type": "text/plain" }))).status).toBe(415);
    expect((await POST(request("not json"))).status).toBe(400);
    for (const body of [null, [], { ...valid, email: "bad" }, { ...valid, use_case: "short" }, { ...valid, use_case: "x".repeat(1001) }, { ...valid, website: {} }]) {
      expect((await POST(request(body))).status).toBe(400);
    }
    expect((await POST(request("x".repeat(4097)))).status).toBe(413);
    expect((await POST(request(valid, { "Content-Length": "4097" }))).status).toBe(413);
    expect(client).not.toHaveBeenCalled();
  });
  it("defers honeypots and duplicate decisions to the database and keeps acceptance generic", async () => {
    const response = await POST(request({ ...valid, website: "spam.example" }));
    expect(response.status).toBe(202); expect(await response.json()).toEqual({ ok: true });
    expect(rpc).toHaveBeenCalledWith("submit_access_request", expect.objectContaining({ honeypot: "spam.example" }));
  });
  it("does not show success when the migration or database is unavailable", async () => {
    for (const result of [{ data: null, error: { message: "missing function secret internal details" } }, { data: null, error: null }, { data: { ok: false }, error: null }]) {
      rpc.mockResolvedValueOnce(result);
      const response = await POST(request());
      expect(response.status).toBe(503); expect(await response.text()).not.toMatch(/secret|function/);
    }
    rpc.mockRejectedValueOnce(new Error("connection failed"));
    expect((await POST(request())).status).toBe(503);
  });
  it("maps database throttle and final-validation failures without leaking details", async () => {
    rpc.mockResolvedValueOnce({ data: null, error: { message: "RATE_LIMITED" } });
    const throttled = await POST(request());
    expect(throttled.status).toBe(429); expect(throttled.headers.get("retry-after")).toBeTruthy();
    rpc.mockResolvedValueOnce({ data: null, error: { message: "INVALID_REQUEST" } });
    expect((await POST(request())).status).toBe(400);
  });
});
