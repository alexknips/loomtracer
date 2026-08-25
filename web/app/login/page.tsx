import { redirect } from "next/navigation";
import { LoginForm } from "@/components/login-form";
import { getOwnerContext } from "@/lib/server/auth";

export const dynamic = "force-dynamic";

export default async function LoginPage({ searchParams }: { searchParams: Promise<{ error?: string }> }) {
  const [owner, params] = await Promise.all([getOwnerContext(), searchParams]);
  if (owner) redirect("/graph");
  return (
    <main className="login-page">
      <LoginForm />
      {params.error === "unauthorized" && (
        <p className="login-notice" role="alert">That Google account is not authorized for this workspace.</p>
      )}
      {params.error === "callback" && (
        <p className="login-notice" role="alert">The sign-in callback could not be completed. Please try again.</p>
      )}
    </main>
  );
}
