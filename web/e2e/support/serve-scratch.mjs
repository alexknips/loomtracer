// Runs a scratch Loomtracer server for the e2e tests: a fresh ledger in a temp directory, seeded with a
// fictional pricing study, served on 127.0.0.1. Playwright starts this as a webServer and stops it
// with SIGTERM; the server is then killed by the pid in the pidfile and its directory removed.
//
// Port: LOOMTRACER_E2E_PORT, default 18736. Never :8080 (the live cell) and never :18732 (the mayor's).
// Nothing here ever reads or writes any ledger but the one it creates.
import { spawn } from "node:child_process";
import { closeSync, mkdtempSync, openSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { SEED_TOKEN, seedDraft, seedLedger } from "./seed-ledger.mjs";

const port = Number(process.env.LOOMTRACER_E2E_PORT ?? 18736);
if (!Number.isInteger(port) || port < 18736 || port === 18732) {
  console.error(`Refusing port ${process.env.LOOMTRACER_E2E_PORT}: the scratch server runs on 18736 or higher, never 8080 or 18732.`);
  process.exit(2);
}

const dir = mkdtempSync(join(tmpdir(), "loomtracer-e2e-"));
const pidFile = join(dir, "loomtracer.pid");
const logFd = openSync(join(dir, "loomtracer.log"), "a");
seedLedger(dir);
seedDraft(dir);

const bin = process.env.LOOMTRACER_BIN || "loomtracer";
const server = spawn(bin, ["serve", "--port", String(port), "--loomtracer-dir", dir], {
  env: { ...process.env, LOOMTRACER_API_KEY: SEED_TOKEN, LOOMTRACER_API_URL: "", LOOMTRACER_DATABASE_URL: "" },
  stdio: ["ignore", logFd, logFd],
});
writeFileSync(pidFile, String(server.pid));
closeSync(logFd);

let stopped = false;
function stop(code = 0) {
  if (stopped) return;
  stopped = true;
  try { process.kill(Number(readFileSync(pidFile, "utf8")), "SIGTERM"); } catch { /* already gone */ }
  rmSync(dir, { recursive: true, force: true });
  process.exit(code);
}
for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) process.on(signal, () => stop(0));
process.on("exit", () => { if (!stopped) { try { process.kill(Number(readFileSync(pidFile, "utf8")), "SIGTERM"); } catch { /* gone */ } } });
server.on("exit", code => { console.error(`The scratch loomtracer server exited (${code}). Log: ${join(dir, "loomtracer.log")}`); stop(code ?? 1); });
console.log(`Scratch Loomtracer server on http://127.0.0.1:${port} (ledger ${dir}, pidfile ${pidFile})`);
