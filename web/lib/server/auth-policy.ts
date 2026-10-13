export function normalizedEmailMatches(email: string | null | undefined, ownerEmail: string): boolean {
  return Boolean(email && email.trim().toLowerCase() === ownerEmail.trim().toLowerCase());
}

export function safeRedirectPath(value: string | null | undefined): string {
  return value?.startsWith("/") && !value.startsWith("//") && !/[\\\u0000-\u001f]/.test(value) ? value : "/graph";
}
// Supabase authorization requests use opaque, URL-safe IDs, not client UUIDs.
export const OAUTH_AUTHORIZATION_ID = /^[A-Za-z0-9_-]{16,256}$/;
