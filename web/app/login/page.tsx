import { redirect } from "next/navigation";
import { LoginForm } from "@/components/login-form";
import { getOwnerContext } from "@/lib/server/auth";
import { safeRedirectPath } from "@/lib/server/auth-policy";

export const dynamic = "force-dynamic";

export default async function LoginPage({ searchParams }: { searchParams: Promise<{ error?: string; next?: string }> }) {
  const [owner, params] = await Promise.all([getOwnerContext(), searchParams]);
  if (owner) redirect(safeRedirectPath(params.next));
  return (
    <main className="login-page">
      <LoginForm next={safeRedirectPath(params.next)} />
      {params.error === "unauthorized" && (
        <p className="login-notice" role="alert">Access is unavailable. New users should reopen their invitation link and choose the invited Google account. Contact the person who invited you if access still fails.</p>
      )}
      {params.error === "callback" && (
        <p className="login-notice" role="alert">The sign-in callback could not be completed. Please try again.</p>
      )}
    </main>
  );
}
