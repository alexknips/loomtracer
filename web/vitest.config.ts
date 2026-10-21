import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "node",
    include: ["**/*.test.ts"],
  },
  resolve: {
    alias: {
      "@reasoning": new URL("../packages/reasoning/src", import.meta.url).pathname,
      "@": new URL(".", import.meta.url).pathname,
    },
  },
});
