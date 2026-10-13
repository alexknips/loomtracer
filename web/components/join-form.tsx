"use client";
import { useState } from "react";
import { createClient } from "@/lib/supabase/client";
export function JoinForm() {
  const [message,setMessage]=useState(""); const [busy,setBusy]=useState(false);
  async function join() {
    setBusy(true); setMessage("");
    try {
      const invitation=window.location.hash.slice(1);
      if (!invitation) throw new Error("Open the complete invitation link you received.");
      const response=await fetch("/api/invitations/prepare",{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify({invitation})});
      if(!response.ok) throw new Error("This invitation link is not valid.");
      window.history.replaceState(null,"","/join");
      const {error}=await createClient().auth.signInWithOAuth({provider:"google",options:{redirectTo:`${window.location.origin}/auth/callback?next=/onboarding`,queryParams:{prompt:"select_account"}}});
      if(error) throw error;
    } catch { setMessage("Could not start sign-up. Reopen your complete invitation link and try again.");setBusy(false); }
  }
  return <div className="login-card"><span className="eyebrow">Your team account</span><h1>Welcome to Loomtracer.</h1>
    <p>Sign in with the Google account that received this invitation. Projects and decisions are shared with your team and the agents its members authorize. Each person uses a separate login.</p>
    <button className="button primary" disabled={busy} onClick={join}>{busy?"Opening Google…":"Accept invitation with Google"}</button>
    {message&&<p role="alert">{message}</p>}<small>Invitations expire after seven days.</small></div>;
}
