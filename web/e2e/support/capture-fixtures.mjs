// Regenerates lib/loomtracer/fixtures/ from a scratch Loomtracer server seeded by seed-ledger.mjs:
//   graph.json                       GET /v1/graph (the whole tenant graph)
//   graph-<project>.json             GET /v1/graph?project=<project>
//   projects.json                    GET /v1/projects
//   decisions.json                   for each decision title, GET /v1/decisions/{id}, its timeline and
//                                    GET /v1/graph?decision={id}
// The ids are fresh on every run, so all of the files are rewritten together. Needs `loomtracer` on PATH
// (or LOOMTRACER_BIN, the binary under test). Port: LOOMTRACER_E2E_PORT, default 18741; never 8080 or 18732.
//   node e2e/support/capture-fixtures.mjs
import { spawn } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { SEED_TOKEN, seedLedger } from "./seed-ledger.mjs";

const port = Number(process.env.LOOMTRACER_E2E_PORT ?? 18741);
if (!Number.isInteger(port) || port < 18736 || port === 18732) {
  console.error(`Refusing port ${process.env.LOOMTRACER_E2E_PORT}: the scratch server runs on 18736 or higher, never 8080 or 18732.`);
  process.exit(2);
}
const out = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "lib", "loomtracer", "fixtures");
const dir = mkdtempSync(join(tmpdir(), "loomtracer-fixtures-"));
const seeded = seedLedger(dir);
const server = spawn(process.env.LOOMTRACER_BIN || "loomtracer", ["serve", "--port", String(port), "--loomtracer-dir", dir], {
  env: { ...process.env, LOOMTRACER_API_KEY: SEED_TOKEN, LOOMTRACER_API_URL: "", LOOMTRACER_DATABASE_URL: "" },
  stdio: "ignore",
});

const base = `http://127.0.0.1:${port}`;
const get = async path => {
  const response = await fetch(`${base}${path}`, { headers: { Authorization: `Bearer ${SEED_TOKEN}` } });
  if (!response.ok) throw new Error(`${path} answered ${response.status}`);
  return response.json();
};
// Keys sorted and the volatile timing zeroed, so a rerun differs only where the ledger does.
const tidy = value => Array.isArray(value) ? value.map(tidy)
  : value && typeof value === "object" ? Object.fromEntries(Object.keys(value).sort().map(key => [key, key === "latency_ms" ? 0 : tidy(value[key])]))
  : value;
const write = (name, value) => writeFileSync(join(out, name), `${JSON.stringify(tidy(value), null, 1)}\n`);

try {
  for (let attempt = 0; ; attempt += 1) {
    try { await get("/v1/health"); break; } catch (error) { if (attempt > 120) throw error; await new Promise(resolve => setTimeout(resolve, 500)); }
  }
  mkdirSync(out, { recursive: true });
  const whole = await get("/v1/graph");
  write("graph.json", whole);
  for (const project of ["pricing", "pricing-experiments"]) write(`graph-${project}.json`, await get(`/v1/graph?project=${project}`));
  write("projects.json", await get("/v1/projects"));
  const details = {};
  for (const decision of whole.decisions) {
    const id = encodeURIComponent(decision.id);
    details[decision.title] = {
      brief: (await get(`/v1/decisions/${id}`)).data,
      timeline: (await get(`/v1/decisions/${id}/timeline`)).data,
      graph: await get(`/v1/graph?decision=${id}`),
    };
  }
  write("decisions.json", details);
  console.log(`Wrote fixtures for ${whole.decisions.length} decisions, ${whole.edges.length} edges, seeded ids ${Object.keys(seeded).join(", ")}`);
} finally {
  server.kill("SIGTERM");
  rmSync(dir, { recursive: true, force: true });
}
