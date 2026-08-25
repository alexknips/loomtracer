export function normalizedEmailMatches(email: string | null | undefined, ownerEmail: string): boolean {
  return Boolean(email && email.trim().toLowerCase() === ownerEmail.trim().toLowerCase());
}

export function safeRedirectPath(value: string | null | undefined): string {
  return value?.startsWith("/") && !value.startsWith("//") ? value : "/graph";
}
