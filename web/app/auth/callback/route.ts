import { NextRequest, NextResponse } from "next/server";
import { bootstrapOwner } from "@/lib/server/auth";
import { createClient } from "@/lib/supabase/server";
import { safeRedirectPath } from "@/lib/server/auth-policy";

export async function GET(request: NextRequest) {
  const code = request.nextUrl.searchParams.get("code");
  const next = safeRedirectPath(request.nextUrl.searchParams.get("next"));
  const origin = request.nextUrl.origin;
  if (!code) return NextResponse.redirect(new URL("/login?error=callback", origin));

  const supabase = await createClient();
  const { error: exchangeError } = await supabase.auth.exchangeCodeForSession(code);
  if (exchangeError) return NextResponse.redirect(new URL("/login?error=callback", origin));
  const { data: { user }, error: userError } = await supabase.auth.getUser();
  if (userError || !user) {
    await supabase.auth.signOut();
    return NextResponse.redirect(new URL("/login?error=callback", origin));
  }
  try {
    await bootstrapOwner(user);
  } catch {
    await supabase.auth.signOut();
    return NextResponse.redirect(new URL("/login?error=unauthorized", origin));
  }
  return NextResponse.redirect(new URL(next, origin));
}
