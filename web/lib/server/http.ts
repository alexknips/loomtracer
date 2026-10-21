import { NextResponse } from "next/server";

export const PRIVATE_HEADERS = {
  "Cache-Control": "private, no-store, max-age=0",
  Vary: "Cookie, Authorization",
  Pragma: "no-cache",
  "X-Content-Type-Options": "nosniff",
};

export function privateJson(body: unknown, init: ResponseInit = {}) {
  return NextResponse.json(body, {
    ...init,
    headers: { ...PRIVATE_HEADERS, ...Object.fromEntries(new Headers(init.headers).entries()) },
  });
}

export function apiError(error: unknown) {
  const message = error instanceof Error ? error.message : typeof error === "object" && error !== null && "message" in error ? String(error.message) : "Unexpected server error.";
  if (message === "UNAUTHORIZED") return privateJson({ error: "Unauthorized" }, { status: 401 });
  if (message === "FORBIDDEN") return privateJson({ error: "Access denied" }, { status: 403 });
  if (message.includes("STALE_REVIEW")) return privateJson({ error: "This entry changed while you were reviewing it. Refresh the page and review the latest version.", code: "STALE_REVIEW" }, { status: 409 });
  return privateJson({ error: "The request could not be completed. Refresh and check your inputs." }, { status: 400 });
}

export function requireSameOrigin(request: Request) {
  const url = new URL(request.url);
  let expectedOrigin = url.origin;
  if (process.env.NODE_ENV === "production") {
    // Proxy-internal URLs and forwarded headers must not define the trust boundary.
    if (!process.env.APP_URL) throw new Error("FORBIDDEN");
    expectedOrigin = new URL(process.env.APP_URL).origin;
  } else if (["localhost", "127.0.0.1", "[::1]"].includes(url.hostname)) {
    // Next's dev server normalizes its URL to localhost even when opened at 127.0.0.1.
    const host = request.headers.get("host");
    if (host) {
      const local = new URL(`${url.protocol}//${host}`);
      if (["localhost", "127.0.0.1", "[::1]"].includes(local.hostname)) expectedOrigin = local.origin;
    }
  }
  if (request.headers.get("origin") !== expectedOrigin) throw new Error("FORBIDDEN");
}
