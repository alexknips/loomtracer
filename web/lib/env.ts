function read(name: string): string {
  const value = process.env[name]?.trim();
  if (!value) throw new Error(`Missing required environment variable: ${name}`);
  return value;
}

export function isDemoMode(): boolean {
  return process.env.NODE_ENV !== "production" && process.env.NEUROFILAMENT_DEMO_MODE === "true";
}

export function publicSupabaseEnv() {
  return {
    url: read("NEXT_PUBLIC_SUPABASE_URL"),
    publishableKey: read("NEXT_PUBLIC_SUPABASE_PUBLISHABLE_KEY"),
  };
}

export function serverEnv() {
  return {
    ...publicSupabaseEnv(),
    secretKey: read("SUPABASE_SECRET_KEY"),
    ownerEmail: read("OWNER_EMAIL").toLowerCase(),
    appUrl: read("APP_URL").replace(/\/$/, ""),
  };
}
