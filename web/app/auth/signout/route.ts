import { cookies } from "next/headers";
import { PROJECT_COOKIE } from "@/lib/projects";
import { NextRequest, NextResponse } from "next/server";
import { createClient } from "@/lib/supabase/server";
import { apiError, requireSameOrigin, PRIVATE_HEADERS } from "@/lib/server/http";
import { appOrigin } from "@/lib/env";

export async function POST(request: NextRequest) {
  try { requireSameOrigin(request); } catch (error) { return apiError(error); }
  const supabase = await createClient();
  await supabase.auth.signOut({ scope: "local" });
  const jar = await cookies();
  jar.delete(PROJECT_COOKIE);
  jar.delete("nf_invitation");
  return NextResponse.redirect(new URL("/login", appOrigin()), { status: 303, headers: { ...PRIVATE_HEADERS, "Clear-Site-Data": '"cache"' } });
}
