import type { Metadata } from "next";
import { PublicHeader } from "@/components/public-header";
import { PublicDemo } from "@/components/public-demo";

export const metadata: Metadata = {
  title: "See the connections — Loomtracer",
  description: "Explore a fictional pricing study: two agents investigate different approaches, and a third connects their findings into a proposal for a team-plan test.",
  robots: { index: true, follow: true },
  alternates: { canonical: "/demo" },
  openGraph: {
    title: "See the connections — Loomtracer",
    description: "Two pricing studies, shared findings, and a proposal to test. Follow the connections in this explicitly fictional collaboration-app example.",
    url: "/demo",
    images: [{ url: "/og.png", alt: "Loomtracer" }],
  },
  twitter: {
    card: "summary_large_image",
    title: "See the connections — Loomtracer",
    description: "Two pricing studies, shared findings, and a proposal to test. Follow the connections in this explicitly fictional collaboration-app example.",
    images: ["/og.png"],
  },
};

export default function DemoPage() {
  return <div className="public-site public-demo-page">
    <a className="public-skip" href="#demo-main">Skip to content</a>
    <PublicHeader demo />
    <main id="demo-main"><PublicDemo /></main>
  </div>;
}
