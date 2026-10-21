import { describe, expect, it, vi } from "vitest";
import { createHiveMindClient, HiveMindError, hivemindConfig } from "./client";

const ok = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
const env = { HIVEMIND_API_URL: "http://127.0.0.1:18736/", HIVEMIND_API_KEY: "secret-token" };

async function failure(promise: Promise<unknown>) {
  try { await promise; } catch (error) { return error as HiveMindError; }
  throw new Error("expected the call to fail");
}

describe("HiveMind client configuration", () => {
  it("needs HIVEMIND_API_URL, and says what to do about it", () => {
    expect(() => hivemindConfig({})).toThrowError(/HIVEMIND_API_URL is not set/);
    expect(() => hivemindConfig({ HIVEMIND_API_URL: "not a url" })).toThrowError(/not a valid URL/);
    expect(() => hivemindConfig({ HIVEMIND_API_URL: "file:///etc/passwd" })).toThrowError(/http:\/\/ or https:\/\//);
  });

  it("trims the URL and treats the token and tenant as optional", () => {
    expect(hivemindConfig({ HIVEMIND_API_URL: " https://cell.example.com/api// " })).toEqual({ baseUrl: "https://cell.example.com/api", token: null, tenant: null });
    expect(hivemindConfig({ ...env, HIVEMIND_TENANT: "acme" })).toEqual({ baseUrl: "http://127.0.0.1:18736", token: "secret-token", tenant: "acme" });
  });
});

describe("HiveMind client requests", () => {
  it("sends the bearer token and tenant, builds the query and unwraps the envelope", async () => {
    const fetchImpl = vi.fn(async () => ok({ result_count: 0, truncated: false, latency_ms: 1, data: { items: [] } }));
    const client = createHiveMindClient({ ...env, HIVEMIND_TENANT: "acme" }, fetchImpl as unknown as typeof fetch);
    expect(await client.search({ q: "seat pricing", status: "proposed", limit: 5 })).toEqual({ items: [] });
    const [url, init] = fetchImpl.mock.calls[0] as unknown as [URL, RequestInit];
    expect(url.toString()).toBe("http://127.0.0.1:18736/v1/decisions/search?q=seat+pricing&status=proposed&limit=5");
    expect(init.headers).toMatchObject({ Authorization: "Bearer secret-token", "X-HiveMind-Tenant": "acme", Accept: "application/json" });
    expect(init.cache).toBe("no-store");
  });

  it("sends no Authorization header to a development-mode server", async () => {
    const fetchImpl = vi.fn(async () => ok({ decisions: [], nodes: [], edges: [] }));
    await createHiveMindClient({ HIVEMIND_API_URL: "http://127.0.0.1:18736" }, fetchImpl as unknown as typeof fetch).graph();
    expect((fetchImpl.mock.calls[0] as unknown as [URL, RequestInit])[1].headers).not.toHaveProperty("Authorization");
  });

  it("asks for one project's graph page, and for the whole graph when it names none", async () => {
    const fetchImpl = vi.fn(async () => ok({ decisions: [], nodes: [], edges: [], limit: 50, total_matches: 0, truncated: false, next_cursor: null }));
    const client = createHiveMindClient(env, fetchImpl as unknown as typeof fetch);
    await client.graph({ project: "personal:agent:claude", limit: 50, cursor: "12:d1" });
    await client.graph();
    const urls = fetchImpl.mock.calls.map(call => String((call as unknown as [URL])[0]));
    expect(urls).toEqual(["http://127.0.0.1:18736/v1/graph?project=personal%3Aagent%3Aclaude&limit=50&cursor=12%3Ad1", "http://127.0.0.1:18736/v1/graph"]);
  });

  it("asks for one decision's graph by its id, whatever the id holds", async () => {
    const fetchImpl = vi.fn(async () => ok({ decisions: [], nodes: [], edges: [] }));
    await createHiveMindClient(env, fetchImpl as unknown as typeof fetch).decisionGraph("capture:42:0");
    expect(String((fetchImpl.mock.calls[0] as unknown as [URL])[0])).toBe("http://127.0.0.1:18736/v1/graph?decision=capture%3A42%3A0");
  });

  it("lists the projects by following the server's pages to the end", async () => {
    const entry = (handle: string) => ({ handle, name: handle, personal: false, parent: null, decisions: { total: 1, proposed: 1, accepted: 0, contested: 0, rejected: 0, superseded: 0 } });
    const pages = [
      { items: [entry("a")], next_cursor: "1", unfiled_decisions: 4 },
      { items: [entry("b")], next_cursor: null, unfiled_decisions: 4 },
    ];
    const fetchImpl = vi.fn(async () => ok({ result_count: 1, truncated: false, latency_ms: 1, data: pages.shift() }));
    const result = await createHiveMindClient(env, fetchImpl as unknown as typeof fetch).projects();
    expect(result.items.map(item => item.handle)).toEqual(["a", "b"]);
    expect(result.unfiled_decisions).toBe(4);
    const urls = fetchImpl.mock.calls.map(call => String((call as unknown as [URL])[0]));
    expect(urls).toEqual(["http://127.0.0.1:18736/v1/projects?limit=100", "http://127.0.0.1:18736/v1/projects?limit=100&cursor=1"]);
  });

  it("refuses a project list that never ends instead of cutting it short", async () => {
    const forever = async () => ok({ result_count: 0, truncated: false, latency_ms: 1, data: { items: [], next_cursor: "more", unfiled_decisions: 0 } });
    const error = await failure(createHiveMindClient(env, forever as unknown as typeof fetch).projects());
    expect(error).toMatchObject({ code: "bad_response" });
  });

  it("scopes a search to a project", async () => {
    const fetchImpl = vi.fn(async () => ok({ result_count: 0, truncated: false, latency_ms: 1, data: { items: [] } }));
    await createHiveMindClient(env, fetchImpl as unknown as typeof fetch).search({ q: "seat", project: "pricing" });
    expect(String((fetchImpl.mock.calls[0] as unknown as [URL])[0])).toBe("http://127.0.0.1:18736/v1/decisions/search?q=seat&project=pricing");
  });

  it("escapes a decision id into its path", async () => {
    const fetchImpl = vi.fn(async () => ok({ data: {} }));
    await createHiveMindClient(env, fetchImpl as unknown as typeof fetch).decision("decision-1/../../v1/graph");
    expect(String((fetchImpl.mock.calls[0] as unknown as [URL])[0])).toBe("http://127.0.0.1:18736/v1/decisions/decision-1%2F..%2F..%2Fv1%2Fgraph");
  });

  it.each([
    [401, "unauthorized", /invalid or missing bearer token Check HIVEMIND_API_KEY/],
    [403, "unauthorized", /Check HIVEMIND_API_KEY/],
    [404, "not_found", /decision not found/],
    [422, "rejected", /decision not found/],
    [500, "server", /decision not found/],
  ] as const)("maps a %s answer to %s", async (status, code, message) => {
    const text = status === 401 ? "invalid or missing bearer token" : "decision not found";
    const client = createHiveMindClient(env, (async () => ok({ error: { code: "x", message: text } }, status)) as unknown as typeof fetch);
    const error = await failure(client.graph());
    expect(error).toBeInstanceOf(HiveMindError);
    expect(error).toMatchObject({ code, status: status >= 500 ? 502 : status });
    expect(error.message).toMatch(message);
  });

  it("reports an unreachable server without leaking the token", async () => {
    const client = createHiveMindClient(env, (async () => { throw new TypeError("fetch failed: Bearer secret-token"); }) as unknown as typeof fetch);
    const error = await failure(client.graph());
    expect(error).toMatchObject({ code: "unreachable", status: 502 });
    expect(error.message).toContain("http://127.0.0.1:18736");
    expect(error.message).not.toContain("secret-token");
  });

  it("refuses a body it cannot read", async () => {
    const client = createHiveMindClient(env, (async () => new Response("<html>proxy error</html>", { status: 200 })) as unknown as typeof fetch);
    expect(await failure(client.graph())).toMatchObject({ code: "bad_response" });
  });

  it("explains a missing configuration at the call, not at import", async () => {
    expect(await failure(createHiveMindClient({}, vi.fn() as unknown as typeof fetch).graph())).toMatchObject({ code: "not_configured", status: 503 });
  });
});
