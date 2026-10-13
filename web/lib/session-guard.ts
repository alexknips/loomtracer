export interface SessionIdentity { userId: string; workspaceId: string; membershipRole: "owner" | "reviewer" }

export function sessionIdentityKey(identity: SessionIdentity): string {
  return JSON.stringify([identity.userId, identity.workspaceId, identity.membershipRole]);
}

/** Keep verified content stable during background checks, while never revealing
 * a hidden view using a response from an earlier account or page lifecycle.
 * userId/loadUserId contain the full session identity key for team-aware callers.
 */
export function createSessionGuard(options: {
  userId: string;
  loadUserId: () => Promise<string | null>;
  hide: () => void;
  show: () => void;
  onInvalid: () => void;
  minCheckIntervalMs?: number;
}) {
  let generation = 0;
  let stopped = false;
  let visible = false;
  let verifiedAt: number | null = null;
  let inFlight: { generation: number; promise: Promise<void> } | null = null;
  const hide = () => {
    generation++;
    visible = false;
    inFlight = null;
    options.hide();
  };
  const invalidate = () => {
    if (stopped) return;
    stopped = true;
    hide();
    options.onInvalid();
  };
  const verify = (): Promise<void> => {
    if (stopped) return Promise.resolve();
    if (inFlight?.generation === generation) return inFlight.promise;
    // Only a visible, verified view can reuse a recent successful check.
    // Resuming a hidden page always requires fresh identity verification.
    const elapsed = verifiedAt === null ? Infinity : Date.now() - verifiedAt;
    if (visible && elapsed >= 0 && elapsed < (options.minCheckIntervalMs ?? 0)) return Promise.resolve();
    if (!visible) options.hide();
    const request = generation;
    let complete!: () => void;
    const promise = new Promise<void>(resolve => { complete = resolve; });
    inFlight = { generation: request, promise };
    void (async () => {
      try {
        const currentUser = await options.loadUserId();
        if (stopped || request !== generation) return;
        if (currentUser !== options.userId) invalidate();
        else {
          verifiedAt = Date.now();
          if (!visible) { visible = true; options.show(); }
        }
      } catch {
        if (!stopped && request === generation) invalidate();
      } finally {
        if (inFlight?.promise === promise) inFlight = null;
        complete();
      }
    })();
    return promise;
  };
  return { hide, invalidate, verify, dispose: () => { stopped = true; hide(); } };
}
