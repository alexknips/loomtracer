"use client";

import { useEffect, useRef, type ReactNode } from "react";
import { PROJECT_COOKIE } from "@/lib/projects";
import { createSessionGuard, sessionIdentityKey, type SessionIdentity } from "@/lib/session-guard";
import { createClient } from "@/lib/supabase/client";

export function SessionBoundary({ userId, workspaceId, membershipRole, demo, children }: SessionIdentity & { demo: boolean; children: ReactNode }) {
  const content = useRef<HTMLDivElement>(null);
  const identityKey = sessionIdentityKey({ userId, workspaceId, membershipRole });
  useEffect(() => {
    if (demo) return;
    const guard = createSessionGuard({
      userId: identityKey,
      loadUserId: async () => {
        const response = await fetch("/api/session", { cache: "no-store" });
        const data = await response.json();
        return response.ok ? sessionIdentityKey(data) : null;
      },
      hide: () => { if (content.current) content.current.style.visibility = "hidden"; },
      show: () => { if (content.current) content.current.style.visibility = "visible"; },
      onInvalid: () => {
        document.cookie = `${PROJECT_COOKIE}=; Path=/; Max-Age=0; SameSite=Lax`;
        window.location.replace("/login");
      },
    });
    const { data: { subscription } } = createClient().auth.onAuthStateChange((event, session) => {
      if (event === "SIGNED_OUT" || (session && session.user.id !== userId)) guard.invalidate();
    });
    const visibility = () => { if (document.hidden) guard.hide(); else void guard.verify(); };
    void guard.verify();
    document.addEventListener("visibilitychange", visibility);
    window.addEventListener("pagehide", guard.hide);
    window.addEventListener("pageshow", guard.verify);
    window.addEventListener("focus", guard.verify);
    // A removed member's visible tab also revalidates without needing navigation.
    const interval = window.setInterval(() => { if (!document.hidden) void guard.verify(); }, 30_000);
    return () => {
      window.clearInterval(interval);
      guard.dispose(); subscription.unsubscribe();
      document.removeEventListener("visibilitychange", visibility);
      window.removeEventListener("pagehide", guard.hide);
      window.removeEventListener("pageshow", guard.verify);
      window.removeEventListener("focus", guard.verify);
    };
  }, [userId, identityKey, demo]);
  return <div ref={content} key={identityKey} style={{ visibility: demo ? "visible" : "hidden" }}>{children}</div>;
}
