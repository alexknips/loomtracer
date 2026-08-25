import { describe, expect, it } from "vitest";
import { addDays } from "@/lib/server/token-time";
import { generatePat, hashTokenSecret, parsePat, secretMatches } from "@/lib/server/tokens";

describe("agent PATs", () => {
  it("generates parseable UUID-strength tokens and stores only a hash", () => {
    const generated = generatePat();
    expect(generated.token).toMatch(/^nf_pat_[0-9a-f-]{36}\.[A-Za-z0-9_-]{43}$/);
    expect(parsePat(generated.token)).toEqual({ tokenId: generated.tokenId, secret: generated.secret });
    expect(generated.secretHash).toBe(hashTokenSecret(generated.secret));
    expect(generated.secretHash).not.toContain(generated.secret);
    expect(secretMatches(generated.secret, generated.secretHash)).toBe(true);
    expect(secretMatches(`${generated.secret.slice(0, -1)}B`, generated.secretHash)).toBe(false);
  });

  it("rejects malformed tokens and calculates supported expiry windows", () => {
    expect(parsePat("nf_pat_bad.secret")).toBeNull();
    expect(addDays(30, new Date("2026-08-24T00:00:00Z"))).toBe("2026-09-23T00:00:00.000Z");
  });
});
