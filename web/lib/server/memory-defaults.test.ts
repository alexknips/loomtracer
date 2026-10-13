import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { pilotFeatureEnabled } from "./pilot-features";
import { boundedBriefsEnabled, withBriefDefaults } from "./memory-defaults";

const request = { project_ref: "project:test", task: "Check the current capture constraints" };

beforeEach(() => {
  vi.stubEnv("NEUROFILAMENT_BRIEF_DELIVERY", undefined);
  vi.stubEnv("NEUROFILAMENT_PILOT_ENABLED", "0");
  vi.stubEnv("NEUROFILAMENT_PILOT_FEATURES", undefined);
});
afterEach(() => vi.unstubAllEnvs());

describe("default bounded briefing", () => {
  it("uses compact delivery and one evidence prefetch without activating pilot features", () => {
    expect(boundedBriefsEnabled()).toBe(true);
    expect(withBriefDefaults(request)).toEqual({
      ...request, delivery: "compact", max_response_bytes: 64000, max_task_evidence_nodes: 1,
    });
    expect(["briefs", "plan-scope", "capture", "review"].map(feature =>
      pilotFeatureEnabled(feature as Parameters<typeof pilotFeatureEnabled>[0]))).toEqual([false, false, false, false]);
    expect(request).not.toHaveProperty("delivery");
  });

  it.each(["", "compact"])("accepts the default delivery setting %j", configured => {
    vi.stubEnv("NEUROFILAMENT_BRIEF_DELIVERY", configured);
    expect(boundedBriefsEnabled()).toBe(true);
  });

  it.each(["legacy", "unknown"])("restores incumbent requests for %s", configured => {
    vi.stubEnv("NEUROFILAMENT_BRIEF_DELIVERY", configured);
    expect(boundedBriefsEnabled()).toBe(false);
    expect(withBriefDefaults(request)).toBe(request);
  });

  it("retains explicit delivery, budgets, zero prefetch, scope and continuation choices", () => {
    const explicit = {
      ...request, delivery: "full" as const, max_response_bytes: 100000, max_linked_nodes: 0,
      max_task_evidence_nodes: 0, workflow_ref: "workflow:maintenance", previous_fingerprint: "f".repeat(64),
      baseline_token: "token", baseline_confirmed: false, allow_delta: false,
    };
    expect(withBriefDefaults(explicit)).toEqual(explicit);
  });
});
