import { createServerClient } from "@supabase/ssr";
import { cookies } from "next/headers";
import { publicSupabaseEnv } from "@/lib/env";

export async function createClient() {
  const cookieStore = await cookies();
  const env = publicSupabaseEnv();
  return createServerClient(env.url, env.publishableKey, {
    global: { fetch: (input, init) => fetch(input, { ...init, cache: "no-store" }) },
    cookies: {
      getAll: () => cookieStore.getAll(),
      setAll(cookiesToSet) {
        try {
          for (const { name, value, options } of cookiesToSet) {
            cookieStore.set(name, value, options);
          }
        } catch {
          // A Server Component cannot mutate cookies. Proxy refreshes sessions.
        }
      },
    },
  });
}
