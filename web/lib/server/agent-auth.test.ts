import { generateKeyPairSync, sign } from "node:crypto";
import { afterEach, describe, expect, it, vi } from "vitest";
import { authenticateAgent, validAgentClaims } from "./agent-auth";
import { OAUTH_AUTHORIZATION_ID, safeRedirectPath } from "./auth-policy";
import { apiError, requireSameOrigin } from "./http";
const uuid = "11111111-1111-4111-8111-111111111111";
const issuer = "https://test.supabase.co/auth/v1", resource = "https://test.example/api/mcp";
const claims = { sub: uuid, client_id: uuid, session_id: uuid, nf_connection_id: uuid, role: "authenticated", iss: issuer, aud: resource, exp: 100 };
afterEach(() => { vi.unstubAllEnvs(); vi.unstubAllGlobals(); });
describe("OAuth resource authorization", () => {
  it("requires the verified principal, issuer, audience and live expiration", () => {
    expect(validAgentClaims(claims, issuer, resource, 99)).toBe(true);
    for (const altered of [{ ...claims, exp: 99 }, { ...claims, aud: "authenticated" }, { ...claims, iss: "https://attacker.example" },
      { ...claims, role: "service_role" }, { ...claims, client_id: undefined }, { ...claims, session_id: "" }, { ...claims, nf_connection_id: null }]) {
      expect(validAgentClaims(altered, issuer, resource, 99)).toBe(false);
    }
  });
  it("rejects legacy PATs and arbitrary bearers before any network access", async () => {
    const spy = vi.spyOn(globalThis, "fetch");
    for (const token of ["nf_pat_11111111-1111-4111-8111-111111111111.secret", "old-secret", "", `${Buffer.from('{"alg":"HS256"}').toString("base64url")}.e30.signature`]) {
      expect(await authenticateAgent(new Request(resource, { headers: { Authorization: `Bearer ${token}` } }))).toBeNull();
    }
    expect(spy).not.toHaveBeenCalled(); spy.mockRestore();
  });
  it("never accepts the fixture credential in production", async () => {
    vi.stubEnv("NODE_ENV", "production"); vi.stubEnv("NEUROFILAMENT_DEMO_MODE", "true");
    expect(await authenticateAgent(new Request(resource, { headers: { Authorization: "Bearer demo-oauth-agent" } }))).toBeNull();
  });
  it("rejects cross-origin consent and invitations", () => {
    expect(() => requireSameOrigin(new Request(resource, { headers: { Origin: "https://attacker.example" } }))).toThrow("FORBIDDEN");
    expect(() => requireSameOrigin(new Request(resource))).toThrow("FORBIDDEN");
    expect(() => requireSameOrigin(new Request(resource, { headers: { Origin: "https://test.example" } }))).not.toThrow();
  });
  it("blocks redirect escapes and preserves a consent continuation", () => {
    for (const next of ["//evil.test", "/\\evil.test", "/\nevil.test", "https://evil.test"]) expect(safeRedirectPath(next)).toBe("/graph");
    expect(safeRedirectPath("/oauth/consent?authorization_id=123")).toBe("/oauth/consent?authorization_id=123");
  });
  it("accepts native opaque authorization IDs without permitting path injection", () => {
    expect(OAUTH_AUTHORIZATION_ID.test("dN0gQRtiLn4DyJoJRbwKV8JwpFbPrtgA")).toBe(true);
    for (const value of ["", "short", "../authorizations/other", "a".repeat(257), "a".repeat(32)+"?action=approve"])
      expect(OAUTH_AUTHORIZATION_ID.test(value)).toBe(false);
  });
  it("uses the canonical production origin despite proxy URL and spoofed forwarded headers", () => {
    vi.stubEnv("NODE_ENV", "production"); vi.stubEnv("APP_URL", "https://test.example");
    const request = (origin: string) => new Request("http://localhost:3000/api/invitations", {
      headers: { origin, host: "attacker.example", "x-forwarded-host": "attacker.example" },
    });
    expect(() => requireSameOrigin(request("https://test.example"))).not.toThrow();
    expect(() => requireSameOrigin(request("https://attacker.example"))).toThrow("FORBIDDEN");
    vi.stubEnv("APP_URL", "");
    expect(() => requireSameOrigin(request("https://test.example"))).toThrow("FORBIDDEN");
  });
  it("accepts the explicit loopback host used by the development browser", () => {
    vi.stubEnv("NODE_ENV", "development");
    expect(() => requireSameOrigin(new Request("http://localhost:3107/api/invitations", {
      headers: { origin: "http://127.0.0.1:3107", host: "127.0.0.1:3107" },
    }))).not.toThrow();
  });
  it("does not expose database row or constraint details in errors", async () => {
    const response = apiError(new Error("FOREIGN-TENANT-SENTINEL secret row"));
    expect(await response.text()).not.toContain("SENTINEL");
    expect(response.headers.get("cache-control")).toContain("private, no-store");
    expect(response.headers.get("vary")).toContain("Authorization");
  });
});

describe("signed OAuth bearer verification", () => {
  it("accepts a signed token with live database authorization and rejects signature tampering", async () => {
    const url = "https://signature-test.supabase.co";
    vi.stubEnv("NEXT_PUBLIC_SUPABASE_URL", url);
    vi.stubEnv("NEXT_PUBLIC_SUPABASE_PUBLISHABLE_KEY", "sb_publishable_test");
    vi.stubEnv("APP_URL", "https://test.example");
    vi.stubEnv("NEUROFILAMENT_DEMO_MODE", "false");
    const keys = generateKeyPairSync("ec", { namedCurve: "prime256v1" });
    const jwk = { ...keys.publicKey.export({ format: "jwk" }), kid: "fixture", alg: "ES256", use: "sig" };
    const encode = (value: unknown) => Buffer.from(JSON.stringify(value)).toString("base64url");
    const payload = { ...claims, iss: `${url}/auth/v1`, exp: Math.floor(Date.now()/1000)+300, iat: Math.floor(Date.now()/1000) };
    const prefix = `${encode({alg:"ES256",typ:"JWT",kid:"fixture"})}.${encode(payload)}`;
    const signature = sign("sha256", Buffer.from(prefix), {key:keys.privateKey,dsaEncoding:"ieee-p1363"}).toString("base64url");
    let databaseCalls = 0;
    vi.stubGlobal("fetch", vi.fn(async (input: RequestInfo | URL) => {
      const path = String(input);
      if(path.includes("/.well-known/jwks.json")) return Response.json({keys:[jwk]});
      if(path.includes("/rpc/agent_request_context")) {
        databaseCalls++;
        return Response.json({connection_id:uuid,user_id:uuid,client_id:uuid,workspace_id:"own",permissions:["context:read"],actor:{id:uuid,kind:"agent",display_name:"Agent"}});
      }
      throw new Error("Unexpected request");
    }));
    const request = (token: string) => new Request(resource,{headers:{Authorization:`Bearer ${token}`}});
    expect((await authenticateAgent(request(`${prefix}.${signature}`)))?.workspaceId).toBe("own");
    expect(databaseCalls).toBe(1);
    const forged = `${prefix.split(".")[0]}.${encode({...payload,sub:"22222222-2222-4222-8222-222222222222"})}.${signature}`;
    expect(await authenticateAgent(request(forged))).toBeNull();
    expect(databaseCalls).toBe(1);
  });
});
