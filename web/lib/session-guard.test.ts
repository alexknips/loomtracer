import { describe, expect, it, vi } from "vitest";
import { createSessionGuard, sessionIdentityKey } from "./session-guard";

function fixture(loadUserId: () => Promise<string | null>) {
  const events = { hide: vi.fn(), show: vi.fn(), onInvalid: vi.fn() };
  return { ...events, guard: createSessionGuard({ userId: "A", loadUserId, ...events }) };
}
function deferred() {
  let resolve!: (id: string | null) => void;
  const promise = new Promise<string | null>(done => { resolve = done; });
  return { promise, resolve };
}
describe("private browser view lifetime", () => {
  it("reveals content only after confirming the same account", async () => {
    const f = fixture(async () => "A");
    await f.guard.verify();
    expect(f.hide).toHaveBeenCalledOnce(); expect(f.show).toHaveBeenCalledOnce();
    expect(f.onInvalid).not.toHaveBeenCalled();
  });
  it.each(["B", null])("clears the view for a different or absent account: %s", async id => {
    const f = fixture(async () => id);
    await f.guard.verify(); await f.guard.verify();
    expect(f.show).not.toHaveBeenCalled(); expect(f.onInvalid).toHaveBeenCalledOnce();
  });
  it("ignores a delayed response after navigating away or hiding the tab", async () => {
    const response = deferred(); const f = fixture(() => response.promise);
    const pending = f.guard.verify(); f.guard.hide(); response.resolve("A"); await pending;
    expect(f.show).not.toHaveBeenCalled();
  });
  it("cannot reveal old content when an earlier response arrives after account switching", async () => {
    const old = deferred(), current = deferred();
    const f = fixture(vi.fn().mockReturnValueOnce(old.promise).mockReturnValueOnce(current.promise));
    const first = f.guard.verify(), second = f.guard.verify();
    current.resolve("B"); await second; old.resolve("A"); await first;
    expect(f.show).not.toHaveBeenCalled(); expect(f.onInvalid).toHaveBeenCalledOnce();
  });
  it("logout invalidates in-flight checks immediately", async () => {
    const response = deferred(); const f = fixture(() => response.promise);
    const pending = f.guard.verify(); f.guard.invalidate(); response.resolve("A"); await pending;
    expect(f.show).not.toHaveBeenCalled(); expect(f.onInvalid).toHaveBeenCalledOnce();
  });
  it("fails closed when verification fails", async () => {
    const f = fixture(async () => { throw new Error("offline"); }); await f.guard.verify();
    expect(f.show).not.toHaveBeenCalled(); expect(f.onInvalid).toHaveBeenCalledOnce();
  });
  it("does not reveal an unmounted view", async () => {
    const response = deferred(); const f = fixture(() => response.promise);
    const pending = f.guard.verify(); f.guard.dispose(); response.resolve("A"); await pending;
    expect(f.show).not.toHaveBeenCalled(); expect(f.onInvalid).not.toHaveBeenCalled();
  });
});


describe("team session identity", () => {
  const member = { userId: "A", workspaceId: "team-a", membershipRole: "reviewer" as const };
  it.each([
    { ...member, workspaceId: "team-b" },
    { ...member, membershipRole: "owner" as const },
  ])("hides cached content when the same user has changed membership context", async identity => {
    const events = { hide: vi.fn(), show: vi.fn(), onInvalid: vi.fn() };
    const guard = createSessionGuard({ userId: sessionIdentityKey(member), loadUserId: async () => sessionIdentityKey(identity), ...events });
    await guard.verify();
    expect(events.show).not.toHaveBeenCalled();
    expect(events.onInvalid).toHaveBeenCalledOnce();
  });
});
