import Link from "next/link";
import type { ReactNode } from "react";

export function AppShell({ children, active }: { children: ReactNode; active: string }) {
  const links = [
    ["Graph", "/graph"],
    ["Review", "/review"],
    ["Agent access", "/settings/tokens"],
  ];
  return (
    <div className="app-shell">
      <header className="topbar">
        <Link className="brand" href="/graph" aria-label="Neurofilament graph home">
          <span className="brand-mark" aria-hidden="true"><i /><i /><i /></span>
          <span>Neurofilament</span>
          <small>Decision intelligence</small>
        </Link>
        <nav aria-label="Primary navigation">
          {links.map(([label, href]) => (
            <Link key={href} href={href} className={active === label ? "active" : undefined}>
              {label}
            </Link>
          ))}
        </nav>
        <form className="owner-chip" action="/auth/signout" method="post">
          <span>JM</span><button type="submit" title="Sign out">Private workspace</button>
        </form>
      </header>
      {children}
    </div>
  );
}
