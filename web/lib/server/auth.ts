import { redirect } from "next/navigation";
import type { User } from "@supabase/supabase-js";
import { isDemoMode } from "@/lib/env";
import { createClient } from "@/lib/supabase/server";

export interface OwnerContext {
  user: Pick<User, "id" | "email">;
  workspaceId: string;
  workspaceName: string;
  membershipRole: "owner" | "reviewer";
  demo: boolean;
  isAdmin: boolean;
}

const DEMO_OWNER: OwnerContext = {
  user: { id: "owner", email: "owner@neurofilament.local" },
  workspaceId: "00000000-0000-0000-0000-000000000001",
  workspaceName: "Personal",
  membershipRole: "owner",
  demo: true,
  isAdmin: true,
};

/** Resolve an admitted human's membership. Login never creates a workspace. */
export async function resolveWorkspace(user: User): Promise<OwnerContext> {
  const supabase = await createClient();
  const { data, error } = await supabase.rpc("get_workspace_context");
  if (error) throw error;
  const workspace = Array.isArray(data) ? data[0] : data;
  if (!workspace || !["owner", "reviewer"].includes(workspace.membership_role)) throw new Error("Workspace membership is unavailable.");
  const { data: isAdmin } = await supabase.rpc("is_beta_admin");
  return {
    user,
    workspaceId: workspace.workspace_id,
    workspaceName: workspace.workspace_name,
    membershipRole: workspace.membership_role,
    demo: false,
    isAdmin: isAdmin === true,
  };
}

export async function getOwnerContext(): Promise<OwnerContext | null> {
  if (isDemoMode()) return DEMO_OWNER;
  const supabase = await createClient();
  const { data: { user }, error } = await supabase.auth.getUser();
  if (error || !user) return null;
  try {
    return await resolveWorkspace(user);
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
