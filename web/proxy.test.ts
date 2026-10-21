import { afterEach, describe, expect, it, vi } from "vitest";
import { NextRequest } from "next/server";
import { proxy } from "./proxy";

afterEach(() => vi.unstubAllEnvs());
const request = (path: string, method = "GET") => new NextRequest(`https://app.example${path}`, { method });

describe("route boundary", () => {
  it.each(["/", "/demo", "/graph", "/search", "/review", "/projects", "/inspect", "/decisions/decision-1", "/api/reasoning"])("lets the full app serve %s", path => {
    expect(proxy(request(path)).status).toBe(200);
  });

  it.each(["/", "/demo", "/og.png", "/favicon.ico", "/reasoning/brand/logo.svg", "/_next/static/chunks/app.js"])("lets a public-only deployment serve %s", path => {
    vi.stubEnv("NEUROFILAMENT_PUBLIC_ONLY", "true");
    expect(proxy(request(path)).status).toBe(200);
  });

  it.each(["/graph", "/search", "/review", "/projects", "/inspect", "/decisions/decision-1", "/api/reasoning", "/api/health"])("hides %s on a public-only deployment", path => {
    vi.stubEnv("NEUROFILAMENT_PUBLIC_ONLY", "true");
    const response = proxy(request(path));
    expect(response.status).toBe(404);
    expect(response.headers.get("cache-control")).toBe("no-store");
  });

  it("serves a public-only deployment's pages to reads only", () => {
    vi.stubEnv("NEUROFILAMENT_PUBLIC_ONLY", "true");
    expect(proxy(request("/", "POST")).status).toBe(404);
  });
});
