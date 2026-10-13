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
      minCheckIntervalMs: 5_000,
      loadUserId: async () => {
        const response = await fetch("/api/session", { cache: "no-store", signal: AbortSignal.timeout(10_000) });
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
    const verifyVisible = () => { if (!document.hidden) void guard.verify(); };
    verifyVisible();
    document.addEventListener("visibilitychange", visibility);
    window.addEventListener("pagehide", guard.hide);
    window.addEventListener("pageshow", verifyVisible);
    window.addEventListener("focus", verifyVisible);
    // A removed member's visible tab also revalidates without needing navigation.
    const interval = window.setInterval(verifyVisible, 30_000);
    return () => {
      window.clearInterval(interval);
      guard.dispose(); subscription.unsubscribe();
      document.removeEventListener("visibilitychange", visibility);
      window.removeEventListener("pagehide", guard.hide);
      window.removeEventListener("pageshow", verifyVisible);
      window.removeEventListener("focus", verifyVisible);
    };
  }, [userId, identityKey, demo]);
  return <div ref={content} key={identityKey} style={{ visibility: demo ? "visible" : "hidden" }}>{children}</div>;
}
