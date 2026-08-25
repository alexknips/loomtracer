import { describe, expect, it } from "vitest";
import { demoGraph } from "@/lib/demo";
import { HostedGraph, rankHostedNodes } from "@/lib/server/hosted-graph";

describe("hosted relevance ranking", () => {
  it("retains active Decision and scope boosts", () => {
    const ranked = rankHostedNodes(demoGraph.nodes, "hosted supabase", 10);
    expect(ranked[0].node.id).toBe("D-hosted");
    expect(ranked[0].score).toBeGreaterThan(ranked.at(-1)?.score ?? 0);
  });

  it("returns no results for stopwords", () => {
    expect(rankHostedNodes(demoGraph.nodes, "the and with")).toEqual([]);
  });

  it("denies capture to read-only PATs before touching persistence", async () => {
    const graph = new HostedGraph({
      tokenId: "token", workspaceId: "workspace", scopes: ["context:read"],
      actor: { id: "actor", display_name: "Reader", kind: "agent", platform: "test" },
    });
    await expect(graph.capture("Decision", { title: "Must not write" })).rejects.toThrow("entries:write is required");
  });
});
