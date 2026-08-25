import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  use: {
    baseURL: "http://127.0.0.1:3000",
    permissions: ["clipboard-read", "clipboard-write"],
  },
  webServer: {
    command: "NEUROFILAMENT_DEMO_MODE=true npm run dev -- --hostname 127.0.0.1",
    url: "http://127.0.0.1:3000/graph",
    reuseExistingServer: true,
  },
});
