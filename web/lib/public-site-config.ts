const PRIVATE_APP_ORIGIN = "https://loomtracer.vercel.app";
const PRIVATE_SIGN_IN_FALLBACK = `${PRIVATE_APP_ORIGIN}/login`;

export function publicSignInUrl() {
  if (process.env.NEUROFILAMENT_PUBLIC_ONLY !== "true") return "/login";
  try {
    const origin = new URL(process.env.PRIVATE_APP_URL ?? PRIVATE_APP_ORIGIN);
    const developmentLoopback = process.env.NODE_ENV !== "production"
      && origin.protocol === "http:" && ["localhost", "127.0.0.1", "[::1]"].includes(origin.hostname);
    if (origin.protocol !== "https:" && !developmentLoopback) return PRIVATE_SIGN_IN_FALLBACK;
    if (origin.username || origin.password) return PRIVATE_SIGN_IN_FALLBACK;
    return new URL("/login", origin).toString();
  } catch {
    return PRIVATE_SIGN_IN_FALLBACK;
  }
}
