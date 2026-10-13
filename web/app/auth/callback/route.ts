import { NextRequest, NextResponse } from "next/server";
import { createServerClient, type CookieOptions } from "@supabase/ssr";
import { safeRedirectPath } from "@/lib/server/auth-policy";
import { PROJECT_COOKIE } from "@/lib/projects";
import { appOrigin, publicSupabaseEnv } from "@/lib/env";
import { PRIVATE_HEADERS } from "@/lib/server/http";

export async function GET(request: NextRequest) {
  const code = request.nextUrl.searchParams.get("code");
  const next = safeRedirectPath(request.nextUrl.searchParams.get("next"));
  // Next's development proxy can normalize 127.0.0.1 to localhost. Redirect
  // to the canonical app origin so host-bound session cookies remain usable.
  const origin = appOrigin();
  if (!code) {
    const response = NextResponse.redirect(new URL("/login?error=callback", origin));
    response.cookies.delete("nf_invitation");
    response.cookies.delete(PROJECT_COOKIE);
    for (const [name, value] of Object.entries(PRIVATE_HEADERS)) response.headers.set(name, value);
    return response;
  }

  // Keep the exchange and membership resolution on one session, then attach every cookie
  // directly to the redirect. Native Auth integration tests exercise this handoff.
  const jar = new Map(request.cookies.getAll().map(cookie => [cookie.name, cookie.value]));
  const changes = new Map<string, { value: string; options: CookieOptions }>();
  const env = publicSupabaseEnv();
  const supabase = createServerClient(env.url, env.publishableKey, {
    global: { fetch: (input, init) => fetch(input, { ...init, cache: "no-store" }) },
    cookies: {
      getAll: () => [...jar].map(([name, value]) => ({ name, value })),
      setAll: cookies => {
        for (const { name, value, options } of cookies) {
          jar.set(name, value); changes.set(name, { value, options });
        }
      },
    },
  });
  function finish(path: string, clearState = false) {
    const response = NextResponse.redirect(new URL(path, origin));
    for (const [name, { value, options }] of changes) response.cookies.set(name, value, options);
    if (clearState) { response.cookies.delete("nf_invitation"); response.cookies.delete(PROJECT_COOKIE); }
    for (const [name, value] of Object.entries(PRIVATE_HEADERS)) response.headers.set(name, value);
    return response;
  }
  const { error: exchangeError } = await supabase.auth.exchangeCodeForSession(code);
  if (exchangeError) return finish("/login?error=callback", true);
  const { data: { user }, error: userError } = await supabase.auth.getUser();
  if (userError || !user) {
    await supabase.auth.signOut({ scope: "local" });
    return finish("/login?error=callback", true);
  }
  try {
    const invitation = jar.get("nf_invitation");
    if (invitation) {
      const [id, secret] = invitation.split(".");
      const { error } = await supabase.rpc("redeem_beta_invitation", { invitation_id: id, invitation_secret: secret });
      if (error) throw error;
    }
    const { error } = await supabase.rpc("get_workspace_context");
    if (error) throw error;
  } catch {
    // Reject this handoff without signing the person out of other valid sessions.
    await supabase.auth.signOut({ scope: "local" });
    return finish("/login?error=unauthorized", true);
  }
  return finish(next, true);
}
