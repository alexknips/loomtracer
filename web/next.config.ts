import type { NextConfig } from "next";
import { fileURLToPath } from "node:url";

const repositoryRoot = fileURLToPath(new URL("../", import.meta.url));

const nextConfig: NextConfig = {
  reactStrictMode: true,
  turbopack: { root: repositoryRoot },
  outputFileTracingRoot: repositoryRoot,
  // Keep the disposable native server and fixture browser tests independent.
  distDir: process.env.NEUROFILAMENT_BUILD_DIR ?? ".next",
  serverExternalPackages: ["@modelcontextprotocol/sdk"],
  async headers() {
    return [
      {
        source: "/:path*",
        headers: [
          { key: "X-Robots-Tag", value: "noindex, nofollow, noarchive" },
          { key: "Referrer-Policy", value: "no-referrer" },
          { key: "X-Frame-Options", value: "DENY" },
        ],
      },
      ...["/", "/demo"].map((source) => ({
        source,
        headers: [{ key: "X-Robots-Tag", value: "index, follow" }],
      })),
    ];
  },
};

export default nextConfig;
