import { AppShell } from "@/components/app-shell";
import { GraphExplorer } from "@/components/graph-explorer";
import { requireOwner } from "@/lib/server/auth";
import { getGraphPayload } from "@/lib/server/data";

export const dynamic = "force-dynamic";

export default async function GraphPage({ searchParams }: { searchParams: Promise<{ root?: string }> }) {
  const [owner, params] = await Promise.all([requireOwner(), searchParams]);
  const payload = await getGraphPayload(owner.workspaceId, params.root);
  return (
    <AppShell active="Graph">
      <GraphExplorer payload={payload} />
    </AppShell>
  );
}
