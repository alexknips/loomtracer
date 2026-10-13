export interface SessionIdentity { userId: string; workspaceId: string; membershipRole: "owner" | "reviewer" }

export function sessionIdentityKey(identity: SessionIdentity): string {
  return JSON.stringify([identity.userId, identity.workspaceId, identity.membershipRole]);
}

/** Prevent a delayed verification from revealing a hidden or previous account view.
 * userId/loadUserId contain the full session identity key for team-aware callers.
 */
export function createSessionGuard(options: {
  userId: string;
  loadUserId: () => Promise<string | null>;
  hide: () => void;
  show: () => void;
  onInvalid: () => void;
}) {
  let generation = 0;
  let stopped = false;
  const hide = () => { generation++; options.hide(); };
  const invalidate = () => {
    if (stopped) return;
    stopped = true;
    hide();
    options.onInvalid();
  };
  const verify = async () => {
    if (stopped) return;
    hide();
    const request = generation;
    try {
      const currentUser = await options.loadUserId();
      if (stopped || request !== generation) return;
      if (currentUser !== options.userId) invalidate();
      else options.show();
    } catch {
      if (!stopped && request === generation) invalidate();
    }
  };
  return { hide, invalidate, verify, dispose: () => { stopped = true; hide(); } };
}
