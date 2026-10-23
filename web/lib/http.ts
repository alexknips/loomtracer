import { NextResponse } from "next/server";
import { LoomtracerError } from "@/lib/loomtracer/client";

/** Everything these routes return is the tenant's private record: never cached, never sniffed. */
export const PRIVATE_HEADERS = {
  "Cache-Control": "private, no-store, max-age=0",
  Pragma: "no-cache",
  "X-Content-Type-Options": "nosniff",
};

export function privateJson(body: unknown, init: ResponseInit = {}) {
  return NextResponse.json(body, { ...init, headers: { ...PRIVATE_HEADERS, ...Object.fromEntries(new Headers(init.headers).entries()) } });
}

/** A Loomtracer failure keeps its meaning; anything else is ours and says nothing about internals. */
export function apiError(error: unknown) {
  if (error instanceof LoomtracerError) return privateJson({ error: error.message, code: error.code }, { status: error.status });
  return privateJson({ error: "The request could not be completed." }, { status: 500 });
}
