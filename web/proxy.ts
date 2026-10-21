import { NextResponse, type NextRequest } from "next/server";
import { refreshSupabaseSession } from "@/lib/supabase/proxy";

export async function proxy(request: NextRequest) {
  const path = request.nextUrl.pathname;
  const publicPage = path === "/" || path === "/demo";
  const localInspector = path === "/inspect";
  const publicIntake = path === "/api/access-requests";
  const publicAsset = path.startsWith("/_next/static/") || path.startsWith("/_next/image")
    || ["/favicon.ico", "/og.png", "/reasoning/brand/logo.svg"].includes(path);
  const publicRequest = (publicPage || publicAsset) && ["GET", "HEAD"].includes(request.method)
    || publicIntake && request.method === "POST";
  // A separate marketing deployment can run against only the additive intake
  // migration. Never invoke private auth or beta handlers on that deployment.
  if (process.env.NEUROFILAMENT_PUBLIC_ONLY === "true") {
    return publicRequest ? NextResponse.next({ request }) : new NextResponse("Not found", {
      status: 404, headers: { "Cache-Control": "no-store", "X-Robots-Tag": "noindex" },
    });
  }
  if (publicPage || localInspector || publicIntake || publicAsset) return NextResponse.next({ request });
  return refreshSupabaseSession(request);
}

export const config = {
  matcher: ["/:path*"],
};
