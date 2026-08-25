"use client";

import { useState } from "react";
import { createClient } from "@/lib/supabase/client";

export function LoginForm() {
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);

  async function signIn() {
    setPending(true);
    setError(null);
    const supabase = createClient();
    const { error: authError } = await supabase.auth.signInWithOAuth({
      provider: "google",
      options: {
        redirectTo: `${window.location.origin}/auth/callback?next=/graph`,
        queryParams: { access_type: "offline", prompt: "consent" },
      },
    });
    if (authError) {
      setError(authError.message);
      setPending(false);
    }
  }

  return (
    <div className="login-card">
      <span className="brand-mark login-mark" aria-hidden="true"><i /><i /><i /></span>
      <span className="eyebrow">Private decision graph</span>
      <h1>Return to the thread.</h1>
      <p>Trace decisions, inspect evidence, and review what your agents propose next.</p>
      <button className="button primary google-button" type="button" onClick={signIn} disabled={pending}>
        <span aria-hidden="true">G</span>{pending ? "Opening Google…" : "Continue with Google"}
      </button>
      {error && <p className="form-error" role="alert">{error}</p>}
      <small>Access is restricted to the configured owner account.</small>
    </div>
  );
}
