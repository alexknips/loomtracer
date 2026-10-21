import Link from "next/link";
import { BrandLogo } from "@/components/brand-logo";

export function PublicHeader({ demo = false }: { demo?: boolean }) {
  return <header className="public-header">
    <Link className="public-wordmark" href="/" aria-label="Loomtracer home">
      <BrandLogo className="public-brand-logo" />
    </Link>
    <nav aria-label="Public navigation">
      <Link className={demo ? "public-nav-current" : undefined} href="/demo">Explore demo</Link>
      {/* The workspace is part of the full app only; a public-only deployment answers 404 for it. */}
      {process.env.NEUROFILAMENT_PUBLIC_ONLY !== "true" && <Link className="public-signin" href="/graph">Open the workspace <span aria-hidden="true">↗</span></Link>}
    </nav>
  </header>;
}
