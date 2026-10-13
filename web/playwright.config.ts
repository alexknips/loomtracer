import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  use: {
    baseURL: "http://127.0.0.1:3107",
    permissions: ["clipboard-read", "clipboard-write"],
  },
  webServer: {
    // Deliberately invalid local fixtures. Discovery and mocked OAuth browser
    // handoffs require configuration, but no test connects to a live database.
    env: { APP_URL: "http://127.0.0.1:3107", NEXT_PUBLIC_SUPABASE_URL: "http://127.0.0.1:54321", NEXT_PUBLIC_SUPABASE_PUBLISHABLE_KEY: "e2e-local-publishable-fixture" },
    command: "NEUROFILAMENT_BUILD_DIR=.next-e2e NEUROFILAMENT_DEMO_MODE=true npm run dev -- --hostname 127.0.0.1 --port 3107",
    url: "http://127.0.0.1:3107/graph",
    reuseExistingServer: false,
  },
});
