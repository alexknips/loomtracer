import { AppShell } from "@/components/app-shell";
import { TokenManager } from "@/components/token-manager";
import { requireOwner } from "@/lib/server/auth";

export const dynamic = "force-dynamic";

export default async function TokensPage() {
  const owner = await requireOwner();
  return <AppShell active="Agent access"><TokenManager workspaceId={owner.workspaceId} /></AppShell>;
}
