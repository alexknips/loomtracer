import { describe, expect, it } from "vitest";
import { demoGraph, getDemoGraph } from "@/lib/demo";
import { HostedGraph, rankHostedNodes } from "@/lib/server/hosted-graph";

describe("hosted relevance ranking", () => {
  it("retains Decision and scope boosts", () => {
    const ranked = rankHostedNodes(demoGraph.nodes, "hosted supabase", 10);
    expect(ranked[0].node.id).toBe("D-hosted-architecture");
    expect(ranked[0].score).toBeGreaterThan(ranked.at(-1)?.score ?? 0);
  });

  it("returns no results for stopwords", () => {
    expect(rankHostedNodes(demoGraph.nodes, "the and with")).toEqual([]);
  });

  it("searches attribution without modeling authors as nodes", () => {
    const ranked = rankHostedNodes(demoGraph.nodes, "Codex", 50);
    expect(ranked.length).toBeGreaterThan(0);
    expect(ranked.every(({ node }) => node.actor?.display_name === "Codex")).toBe(true);
    expect(demoGraph.nodes.some((node) => node.type === ("Actor" as never))).toBe(false);
    expect(demoGraph.edges.some((edge) => ["made_by", "ratified_by"].includes(edge.edge_type))).toBe(false);
  });

  it("returns a bounded demo neighborhood and can expand it", () => {
    const focused = getDemoGraph("ART-project-chronicle", { depth: 1 });
    const expanded = getDemoGraph("ART-project-chronicle", { depth: 2 });
    expect(focused.nodes.length).toBeLessThan(demoGraph.nodes.length);
    expect(expanded.nodes.length).toBeGreaterThan(focused.nodes.length);
    expect(focused.depth).toBe(1);
    expect(expanded.depth).toBe(2);
  });

  it("shows the complete graph by default and reports corpus totals", () => {
    const complete = getDemoGraph();
    expect(complete.view).toBe("all");
    expect(complete.nodes).toHaveLength(demoGraph.nodes.length);
    expect(complete.edges).toHaveLength(demoGraph.edges.length);
    expect(complete.total_nodes).toBe(demoGraph.nodes.length);
    expect(complete.total_edges).toBe(demoGraph.edges.length);
  });

  it("denies capture to read-only agents before touching persistence", async () => {
    const graph = new HostedGraph({
      userId: "user", clientId: "client", accessToken: "test", connectionId: "token", workspaceId: "workspace", scopes: ["context:read"],
      actor: { id: "actor", display_name: "Reader", kind: "agent", platform: "test" },
    });
    await expect(graph.capture("Decision", { title: "Must not write" })).rejects.toThrow("proposal permission is required");
  });
});
