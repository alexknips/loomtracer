import { notFound } from "next/navigation";
import { AppShell } from "@/components/app-shell";
import { AccessRequestManager } from "@/components/access-request-manager";
import { requireOwner } from "@/lib/server/auth";
import { createClient } from "@/lib/supabase/server";

export const dynamic = "force-dynamic";
export default async function AccessRequestsPage() {
  const context = await requireOwner();
  if (!context.isAdmin) notFound();
  const { data, error } = context.demo ? { data: [], error: null } : await (await createClient()).rpc("list_access_requests");
  if (error) throw new Error("Could not load access requests. Please refresh.");
  return <AppShell active="Access requests"><AccessRequestManager requests={data ?? []} demo={context.demo} /></AppShell>;
}
