import Link from "next/link";
import type { ReactNode } from "react";
import { BrandLogo } from "@/components/brand-logo";
import { hivemind } from "@/lib/hivemind/client";

const LINKS = [
  ["Reasoning", "/graph"],
  ["Search", "/search"],
  ["Review", "/review"],
  ["Projects", "/projects"],
  ["Inspect file", "/inspect"],
];

export async function AppShell({ children, active }: { children: ReactNode; active: string }) {
  // The tenant and actor the server resolves for this workspace's key: what everything below is read as.
  const who = await hivemind().whoami().catch(() => null);
  return (
    <div className="app-shell">
      <header className="topbar">
        <Link className="brand" href="/graph" aria-label="Loomtracer graph home">
          <BrandLogo className="app-brand-logo" />
          <span className="brand-identity">
            <span className="workspace-identity" data-testid="workspace-identity" title={who ? `Reading tenant ${who.tenant_id} as ${who.actor_id}` : "HiveMind is not connected"}>
              {who ? `${who.tenant_id} · ${who.actor_id}` : "HiveMind not connected"}
            </span>
          </span>
        </Link>
        <nav aria-label="Primary navigation">
          {LINKS.map(([label, href]) => (
            <Link key={href} href={href} className={active === label ? "active" : undefined}>
              {label}
            </Link>
          ))}
        </nav>
        <span />
      </header>
      {children}
    </div>
  );
}
