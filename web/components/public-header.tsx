import Link from "next/link";
import { publicSignInUrl } from "@/lib/public-site-config";
import { BrandLogo } from "@/components/brand-logo";

export function PublicHeader({ demo = false }: { demo?: boolean }) {
  return <header className="public-header">
    <Link className="public-wordmark" href="/" aria-label="Loomtracer home">
      <BrandLogo className="public-brand-logo" />
    </Link>
    <nav aria-label="Public navigation">
      <Link className={demo ? "public-nav-current" : undefined} href="/demo">Explore demo</Link>
      <Link className="public-signin" href={publicSignInUrl()} prefetch={false}>Sign in <span aria-hidden="true">↗</span></Link>
    </nav>
  </header>;
}
