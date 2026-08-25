import { describe, expect, it } from "vitest";
import { normalizedEmailMatches, safeRedirectPath } from "@/lib/server/auth-policy";

describe("owner authentication policy", () => {
  it("normalizes the owner email but rejects every other account", () => {
    expect(normalizedEmailMatches(" Owner@Example.com ", "owner@example.com")).toBe(true);
    expect(normalizedEmailMatches("other@example.com", "owner@example.com")).toBe(false);
    expect(normalizedEmailMatches(null, "owner@example.com")).toBe(false);
  });

  it("allows only app-relative callback destinations", () => {
    expect(safeRedirectPath("/review")).toBe("/review");
    expect(safeRedirectPath("https://evil.example")).toBe("/graph");
    expect(safeRedirectPath("//evil.example")).toBe("/graph");
  });
});
