import { defineConfig } from "@playwright/test";

// A scratch `hivemind serve` (e2e/support/serve-scratch.mjs) holds a seeded, fictional ledger. The
// UI runs against it; no test reads or writes the live cell. The port is 18736 or higher.
const hivemindPort = Number(process.env.HIVEMIND_E2E_PORT ?? 18736);
const hivemindUrl = `http://127.0.0.1:${hivemindPort}`;
const webUrl = "http://127.0.0.1:3107";
const SEED_TOKEN = "e2e-scratch-token";
// LOOMTRACER_E2E_BUILT=1 tests the production build (`next build`, then `next start`) instead of `next dev`.
const built = process.env.LOOMTRACER_E2E_BUILT === "1";
const buildDir = "NEUROFILAMENT_BUILD_DIR=.next-e2e";

export default defineConfig({
  testDir: "./e2e",
  workers: 1,
  // `next dev` compiles each route on first visit.
  timeout: 90_000,
  expect: { timeout: 20_000 },
  use: { baseURL: webUrl },
  webServer: [
    {
      command: "node e2e/support/serve-scratch.mjs",
      env: { HIVEMIND_E2E_PORT: String(hivemindPort) },
      url: `${hivemindUrl}/v1/health`,
      reuseExistingServer: false,
      timeout: 120_000,
    },
    {
      command: built
        ? `${buildDir} npm run build && ${buildDir} npm run start -- --hostname 127.0.0.1 --port 3107`
        : `${buildDir} npm run dev -- --hostname 127.0.0.1 --port 3107`,
      env: { HIVEMIND_API_URL: hivemindUrl, HIVEMIND_API_KEY: SEED_TOKEN },
      url: `${webUrl}/api/health`,
      reuseExistingServer: false,
      timeout: 240_000,
    },
  ],
});
