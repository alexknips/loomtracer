import Link from "next/link";
import { BrandLogo } from "@/components/brand-logo";
import type { ReactNode } from "react";
import { requireOwner } from "@/lib/server/auth";
import { SessionBoundary } from "@/components/session-boundary";
import styles from "@/components/workspace-administration.module.css";

export async function AppShell({ children, active }: { children: ReactNode; active: string }) {
  const owner = await requireOwner();
  const links = [
    ["Reasoning", "/graph"],
    ["Inspect file", "/inspect"],
    ["Projects", "/projects"],

    ["Agent connections", "/settings/connections"],
    ["Team", "/settings/team"],
  ];
  if (owner.isAdmin) links.push(["Invitations", "/settings/invitations"], ["Access requests", "/settings/access-requests"]);
  return (
    <SessionBoundary userId={owner.user.id} workspaceId={owner.workspaceId} membershipRole={owner.membershipRole} demo={owner.demo}><div className="app-shell">
      <header className="topbar">
        <Link className={`brand ${styles.brand}`} href="/graph" aria-label="Loomtracer graph home">
          <BrandLogo className="app-brand-logo" />
          <span className={styles.brandIdentity}>
            <span className={styles.workspaceIdentity} aria-label={`Current workspace: ${owner.workspaceName}`} title={owner.workspaceName}>{owner.workspaceName} · {owner.membershipRole === "owner" ? "Owner" : "Member"}</span>
          </span>
        </Link>
        <nav aria-label="Primary navigation">
          {links.map(([label, href]) => (
            <Link key={href} href={href} className={active === label ? "active" : undefined}>
              {label}
            </Link>
          ))}
        </nav>
        <form className="owner-chip" action="/auth/signout" method="post">
          <span>{owner.user.email?.slice(0, 2).toUpperCase() ?? "ME"}</span><button type="submit" aria-label={`Sign out of ${owner.user.email ?? "your account"}`} title={owner.workspaceName}>Sign out</button>
        </form>
      </header>
      {children}
    </div></SessionBoundary>
  );
}
