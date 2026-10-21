import { appOrigin } from "@/lib/env";
import { z } from "zod";
import { apiError, privateJson } from "@/lib/server/http";

export const workspaceNameSchema = z.string().trim().min(1).max(80);
export const invitedEmailSchema = z.string().trim().toLowerCase().pipe(z.email().max(254));

/** These fixed messages are safe to show without returning raw database errors. */
export function invitationError(error: unknown) {
  const message = error instanceof Error ? error.message
    : typeof error === "object" && error !== null && "message" in error ? String(error.message) : "";
  if (message === "this identity already belongs to a workspace") {
    return privateJson({ error: "This person already belongs to a workspace. Each person can join one workspace during the pilot." }, { status: 409 });
  }
  if (message === "an invitation is already pending") {
    return privateJson({ error: "This person already has a pending invitation to another workspace. Revoke that invitation before creating a replacement." }, { status: 409 });
  }
  return apiError(error);
}

export function invitationLink(invitation: { id: string; secret: string }): string {
  return new URL(`/join#${invitation.id}.${invitation.secret}`, appOrigin()).href;
}
