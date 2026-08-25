import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  metadataBase: new URL(process.env.APP_URL ?? "http://localhost:3000"),
  title: "Neurofilament — Decision Intelligence",
  description: "A private, provenance-aware decision graph for AI agents.",
  robots: {
    index: false,
    follow: false,
    nocache: true,
  },
  openGraph: {
    title: "Neurofilament",
    description: "Decision intelligence for AI agents",
    images: [{ url: "/og.png", width: 1731, height: 909, alt: "Neurofilament decision graph" }],
  },
  twitter: {
    card: "summary_large_image",
    title: "Neurofilament",
    description: "Decision intelligence for AI agents",
    images: ["/og.png"],
  },
};

export default function RootLayout({ children }: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
