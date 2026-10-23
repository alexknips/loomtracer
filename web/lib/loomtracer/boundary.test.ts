import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";

const root = new URL("../..", import.meta.url).pathname;
const SKIP = new Set(["node_modules", ".next", ".next-e2e", "e2e", "test-results", "playwright-report"]);
function sources(dir: string): string[] {
  return readdirSync(dir).flatMap(name => {
    if (SKIP.has(name) || name.startsWith(".next")) return [];
    const path = join(dir, name);
    return statSync(path).isDirectory() ? sources(path) : /\.(ts|tsx|mjs)$/.test(name) && !/\.test\.|\.d\.ts$/.test(name) ? [path] : [];
  });
}
const files = sources(root).map(path => ({ path: relative(root, path), text: readFileSync(path, "utf8") }));

describe("the bearer token stays on the server", () => {
  it("is never imported by a client component", () => {
    const clients = files.filter(file => /^\s*['"]use client['"]/.test(file.text));
    expect(clients.length).toBeGreaterThan(0);
    const leaks = clients.filter(file => /loomtracer\/(client|bundle)['"]|node:crypto/.test(file.text)).map(file => file.path);
    expect(leaks).toEqual([]);
  });

  it("is read from the environment only in lib/loomtracer/client.ts, and never from a NEXT_PUBLIC variable", () => {
    const reads = files.filter(file => /\benv\.LOOMTRACER_API_KEY|env\[["']LOOMTRACER_API_KEY/.test(file.text)).map(file => file.path);
    expect(reads).toEqual(["lib/loomtracer/client.ts"]);
    expect(files.filter(file => /NEXT_PUBLIC_[A-Z_]*(LOOMTRACER|TOKEN|KEY)/.test(file.text)).map(file => file.path)).toEqual([]);
  });

  it("is not named in the Next config, which would inline it into the client bundle", () => {
    expect(readFileSync(join(root, "next.config.ts"), "utf8")).not.toMatch(/LOOMTRACER|env:/);
  });
});
