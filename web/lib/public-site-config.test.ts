import { afterEach, describe, expect, it, vi } from "vitest";
import { publicSignInUrl } from "./public-site-config";

afterEach(() => vi.unstubAllEnvs());
describe("public site sign-in destination", () => {
  it("stays local for the combined app and uses the private origin for a public-only deployment", () => {
    vi.stubEnv("NEUROFILAMENT_PUBLIC_ONLY", "false");
    expect(publicSignInUrl()).toBe("/login");
    vi.stubEnv("NEUROFILAMENT_PUBLIC_ONLY", "true");
    vi.stubEnv("PRIVATE_APP_URL", "https://private.example/subpath");
    expect(publicSignInUrl()).toBe("https://private.example/login");
  });
  it("rejects insecure or malformed operator destinations in production", () => {
    vi.stubEnv("NEUROFILAMENT_PUBLIC_ONLY", "true");
    vi.stubEnv("NODE_ENV", "production");
    for (const value of ["javascript:alert(1)", "http://example.com", "https://user:password@example.com", "bad"]) {
      vi.stubEnv("PRIVATE_APP_URL", value);
      expect(publicSignInUrl()).toBe("https://loomtracer.vercel.app/login");
    }
  });
  it("uses the canonical Loomtracer origin when no private destination is configured", () => {
    vi.stubEnv("NEUROFILAMENT_PUBLIC_ONLY", "true");
    vi.stubEnv("PRIVATE_APP_URL", undefined);
    expect(publicSignInUrl()).toBe("https://loomtracer.vercel.app/login");
  });
});
