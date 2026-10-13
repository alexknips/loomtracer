import type { Metadata } from "next";
import localFont from "next/font/local";
import "./globals.css";

const fraunces = localFont({ src: "./fonts/fraunces-latin.woff2", variable: "--font-headline", weight: "600", style: "normal", display: "swap" });
const sourceSans = localFont({ src: "./fonts/source-sans-3-latin.woff2", variable: "--font-body", weight: "400 600", style: "normal", display: "swap" });

export const metadata: Metadata = {
  metadataBase: new URL(process.env.APP_URL ?? "http://localhost:3000"),
  title: "Loomtracer — Shared project memory",
  description: "A private, provenance-aware decision graph for AI agents.",
  robots: {
    index: false,
    follow: false,
    nocache: true,
  },
  openGraph: {
    title: "Loomtracer — Connect the dots.",
    description: "Shared project memory, from open questions to informed decisions.",
    images: [{ url: "/og.png", width: 1731, height: 909, alt: "Loomtracer — Connect the dots." }],
  },
  twitter: {
    card: "summary_large_image",
    title: "Loomtracer — Connect the dots.",
    description: "Shared project memory, from open questions to informed decisions.",
    images: ["/og.png"],
  },
};

export default function RootLayout({ children }: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="en" className={`${fraunces.variable} ${sourceSans.variable}`}>
      <body>{children}</body>
    </html>
  );
}
