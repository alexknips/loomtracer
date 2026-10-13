"use client";

import { useState, type FormEvent } from "react";

export function PublicAccessForm() {
  const [state, setState] = useState<"idle" | "pending" | "success" | "error">("idle");
  const [message, setMessage] = useState("");

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = new FormData(event.currentTarget);
    setState("pending");
    setMessage("");
    try {
      const response = await fetch("/api/access-requests", {
        method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ email: form.get("email"), use_case: form.get("use_case"), website: form.get("website") }),
      });
      const result = await response.json().catch(() => null);
      if (!response.ok || result?.ok !== true) {
        setState("error");
        setMessage(response.status === 429
          ? "Requests are busy right now. Please try again in a little while."
          : response.status === 400
            ? "Check your email and add 10–1,000 characters about what you want to build."
            : "We couldn't save your request. Please try again shortly.");
        return;
      }
      setState("success");
    } catch {
      setState("error");
      setMessage("We couldn't save your request. Check your connection and try again.");
    }
  }

  if (state === "success") return <div className="public-form-success" role="status">
    <span aria-hidden="true">✓</span><h3>Your request is in the queue.</h3>
    <p>We&apos;ll contact you about access. The private beta is opening to a small group of builders.</p>
  </div>;

  return <form className="public-access-form" onSubmit={submit}>
    <label htmlFor="access-email">Email address</label>
    <input id="access-email" name="email" type="email" autoComplete="email" placeholder="you@yourteam.com" maxLength={254} required disabled={state === "pending"} />
    <label htmlFor="access-use-case">What would you like your agents to remember?</label>
    <textarea id="access-use-case" name="use_case" placeholder="Tell us a little about your project and the agents you work with." minLength={10} maxLength={1000} rows={4} required disabled={state === "pending"} />
    <div className="public-form-trap" aria-hidden="true"><label htmlFor="access-website">Leave this field empty</label><input id="access-website" name="website" tabIndex={-1} autoComplete="off" /></div>
    <button className="public-button public-button-dark" type="submit" disabled={state === "pending"}>{state === "pending" ? "Sending request…" : "Request access"}<span aria-hidden="true">↗</span></button>
    <p className="public-form-note">Your email and project description are used to review your request and contact you about Loomtracer access. Please leave out sensitive information.</p>
    {message && <p className="public-form-error" role="alert">{message}</p>}
  </form>;
}
