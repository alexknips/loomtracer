import { NextResponse } from "next/server";

export const PRIVATE_HEADERS = {
  "Cache-Control": "no-store, max-age=0",
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
  const message = error instanceof Error ? error.message : "Unexpected server error.";
  if (message === "UNAUTHORIZED") return privateJson({ error: "Unauthorized" }, { status: 401 });
  return privateJson({ error: message }, { status: 400 });
}
