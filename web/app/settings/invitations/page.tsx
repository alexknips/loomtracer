import { notFound } from "next/navigation";
import { AppShell } from "@/components/app-shell";
import { InvitationManager } from "@/components/invitation-manager";
import { requireOwner } from "@/lib/server/auth";
import { createClient } from "@/lib/supabase/server";
export const dynamic="force-dynamic";
export default async function InvitationsPage() {
  const owner=await requireOwner();if(!owner.isAdmin)notFound();
  const {data,error}=owner.demo?{data:{invitations:[],accounts:[],enabled:false},error:null}:await(await createClient()).rpc("list_beta_invitations");
  if(error)throw new Error("Could not load invitations.");
  return <AppShell active="Invitations"><InvitationManager {...data} demo={owner.demo}/></AppShell>;
}
