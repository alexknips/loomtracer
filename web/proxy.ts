import { NextResponse, type NextRequest } from "next/server";

/**
 * The workspace has no login of its own: it shows whatever the Loomtracer server behind
 * LOOMTRACER_API_URL lets its bearer token read. Run it on a loopback address or behind your own
 * access control. A separate public deployment (NEUROFILAMENT_PUBLIC_ONLY=true) serves only the
 * landing page, the demo and their assets, and answers 404 to everything else.
 */
export function proxy(request: NextRequest) {
  if (process.env.NEUROFILAMENT_PUBLIC_ONLY !== "true") return NextResponse.next({ request });
  const path = request.nextUrl.pathname;
  const publicPage = path === "/" || path === "/demo";
  const publicAsset = path.startsWith("/_next/static/") || path.startsWith("/_next/image")
    || ["/favicon.ico", "/og.png", "/reasoning/brand/logo.svg"].includes(path);
  if ((publicPage || publicAsset) && ["GET", "HEAD"].includes(request.method)) return NextResponse.next({ request });
  return new NextResponse("Not found", { status: 404, headers: { "Cache-Control": "no-store", "X-Robots-Tag": "noindex" } });
}

export const config = {
  matcher: ["/:path*"],
};
