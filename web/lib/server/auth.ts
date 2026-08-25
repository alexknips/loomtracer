import { redirect } from "next/navigation";
import type { User } from "@supabase/supabase-js";
import { isDemoMode, serverEnv } from "@/lib/env";
import { createClient } from "@/lib/supabase/server";
import { normalizedEmailMatches } from "@/lib/server/auth-policy";

export interface OwnerContext {
  user: Pick<User, "id" | "email">;
  workspaceId: string;
  workspaceName: string;
  demo: boolean;
}

const DEMO_OWNER: OwnerContext = {
  user: { id: "owner", email: "owner@neurofilament.local" },
  workspaceId: "00000000-0000-0000-0000-000000000001",
  workspaceName: "Personal",
  demo: true,
};

export async function bootstrapOwner(user: User): Promise<OwnerContext> {
  const env = serverEnv();
  if (!normalizedEmailMatches(user.email, env.ownerEmail)) throw new Error("This account is not authorized.");
  const supabase = await createClient();
  const { data, error } = await supabase.rpc("bootstrap_personal_workspace");
  if (error) throw error;
  const workspace = Array.isArray(data) ? data[0] : data;
  if (!workspace) throw new Error("Workspace bootstrap returned no workspace.");
  return {
    user,
    workspaceId: workspace.workspace_id,
    workspaceName: workspace.workspace_name,
    demo: false,
  };
}

export async function getOwnerContext(): Promise<OwnerContext | null> {
  if (isDemoMode()) return DEMO_OWNER;
  const supabase = await createClient();
  const { data: { user }, error } = await supabase.auth.getUser();
  if (error || !user) return null;
  try {
    return await bootstrapOwner(user);
  } catch {
    return null;
  }
}

export async function requireOwner(): Promise<OwnerContext> {
  const owner = await getOwnerContext();
  if (!owner) redirect("/login");
  return owner;
}

export async function requireOwnerApi(): Promise<OwnerContext> {
  const owner = await getOwnerContext();
  if (!owner) throw new Error("UNAUTHORIZED");
  return owner;
}
