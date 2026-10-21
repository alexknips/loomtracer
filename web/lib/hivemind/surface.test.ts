import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const root = new URL("../..", import.meta.url).pathname;
function sources(dir: string): string[] {
  return readdirSync(dir).flatMap(name => {
    const path = join(dir, name);
    return statSync(path).isDirectory() ? sources(path) : /\.tsx$/.test(name) ? [path] : [];
  });
}

describe("the UI has no login, so it is a loopback surface", () => {
  it("binds 127.0.0.1 for dev and start", () => {
    const { scripts } = JSON.parse(readFileSync(join(root, "package.json"), "utf8")) as { scripts: Record<string, string> };
    expect(scripts.dev).toMatch(/\bnext dev\b.*(-H|--hostname) 127\.0\.0\.1\b/);
    expect(scripts.start).toMatch(/\bnext start\b.*(-H|--hostname) 127\.0\.0\.1\b/);
  });
});

describe("no dead buttons", () => {
  it("never links to an in-page anchor that nothing defines", () => {
    const text = ["app", "components"].flatMap(dir => sources(join(root, dir))).map(path => readFileSync(path, "utf8")).join("\n");
    const linked = [...text.matchAll(/href=["']\/?#([\w-]+)["']/g)].map(match => match[1]);
    const defined = new Set([...text.matchAll(/\bid=["']([\w-]+)["']/g)].map(match => match[1]));
    expect(linked.filter(id => !defined.has(id))).toEqual([]);
  });
});
