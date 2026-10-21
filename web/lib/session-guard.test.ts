import { afterEach, describe, expect, it, vi } from "vitest";
import { createSessionGuard, sessionIdentityKey } from "./session-guard";

function fixture(loadUserId: () => Promise<string | null>, minCheckIntervalMs = 0) {
  const events = { hide: vi.fn(), show: vi.fn(), onInvalid: vi.fn() };
  return { ...events, guard: createSessionGuard({ userId: "A", loadUserId, minCheckIntervalMs, ...events }) };
}
afterEach(() => vi.restoreAllMocks());
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
  it("keeps an already verified view visible while a routine check is pending", async () => {
    const response = deferred();
    const load = vi.fn().mockResolvedValueOnce("A").mockReturnValueOnce(response.promise);
    const f = fixture(load);
    await f.guard.verify(); f.hide.mockClear(); f.show.mockClear();
    const pending = f.guard.verify(), overlap = f.guard.verify();
    expect(load).toHaveBeenCalledTimes(2);
    expect(f.hide).not.toHaveBeenCalled();
    response.resolve("A"); await Promise.all([pending, overlap]);
    expect(f.hide).not.toHaveBeenCalled();
    expect(f.onInvalid).not.toHaveBeenCalled();
  });
  it("coalesces overlapping initial, focus, and timer checks", async () => {
    const response = deferred(); const load = vi.fn(() => response.promise);
    const f = fixture(load);
    const first = f.guard.verify(), second = f.guard.verify(), third = f.guard.verify();
    expect(load).toHaveBeenCalledOnce();
    expect(f.show).not.toHaveBeenCalled();
    response.resolve("A"); await Promise.all([first, second, third]);
    expect(f.show).toHaveBeenCalledOnce();
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
    const first = f.guard.verify(); f.guard.hide(); const second = f.guard.verify();
    current.resolve("B"); await second; old.resolve("A"); await first;
    expect(f.show).not.toHaveBeenCalled(); expect(f.onInvalid).toHaveBeenCalledOnce();
  });
  it("does not let a stale quiet check reveal a hidden view or replace its resume check", async () => {
    const old = deferred(), current = deferred();
    const load = vi.fn().mockResolvedValueOnce("A").mockReturnValueOnce(old.promise).mockReturnValueOnce(current.promise);
    const f = fixture(load);
    await f.guard.verify(); f.show.mockClear();
    const background = f.guard.verify(); f.guard.hide(); const resumed = f.guard.verify();
    old.resolve("A"); await background;
    expect(f.show).not.toHaveBeenCalled();
    const overlap = f.guard.verify();
    expect(load).toHaveBeenCalledTimes(3);
    current.resolve("A"); await Promise.all([resumed, overlap]);
    expect(f.show).toHaveBeenCalledOnce();
    expect(f.onInvalid).not.toHaveBeenCalled();
  });
  it.each(["B", null])("hides a visible view if a routine check finds changed or revoked access: %s", async id => {
    const response = deferred();
    const f = fixture(vi.fn().mockResolvedValueOnce("A").mockReturnValueOnce(response.promise));
    await f.guard.verify(); f.hide.mockClear(); f.show.mockClear();
    const pending = f.guard.verify();
    expect(f.hide).not.toHaveBeenCalled();
    response.resolve(id); await pending;
    expect(f.hide).toHaveBeenCalledOnce();
    expect(f.show).not.toHaveBeenCalled();
    expect(f.onInvalid).toHaveBeenCalledOnce();
  });
  it("throttles recently verified visible checks but always rechecks a hidden view", async () => {
    let now = 1_000;
    vi.spyOn(Date, "now").mockImplementation(() => now);
    const load = vi.fn(async () => "A"); const f = fixture(load, 5_000);
    await f.guard.verify(); f.hide.mockClear();
    now += 4_999; await f.guard.verify();
    expect(load).toHaveBeenCalledOnce();
    expect(f.hide).not.toHaveBeenCalled();
    now += 1; await f.guard.verify();
    expect(load).toHaveBeenCalledTimes(2);
    expect(f.hide).not.toHaveBeenCalled();
    f.guard.hide(); await f.guard.verify();
    expect(load).toHaveBeenCalledTimes(3);
    expect(f.onInvalid).not.toHaveBeenCalled();
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
  it("fails closed when routine verification of a visible view fails", async () => {
    const f = fixture(vi.fn().mockResolvedValueOnce("A").mockRejectedValueOnce(new Error("offline")));
    await f.guard.verify(); f.hide.mockClear(); f.show.mockClear();
    await f.guard.verify();
    expect(f.hide).toHaveBeenCalledOnce(); expect(f.show).not.toHaveBeenCalled();
    expect(f.onInvalid).toHaveBeenCalledOnce();
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
  it.each([
    { ...member, workspaceId: "team-b" },
    { ...member, membershipRole: "owner" as const },
  ])("invalidates an already visible view when workspace or role changes", async identity => {
    const events = { hide: vi.fn(), show: vi.fn(), onInvalid: vi.fn() };
    const loadUserId = vi.fn().mockResolvedValueOnce(sessionIdentityKey(member)).mockResolvedValueOnce(sessionIdentityKey(identity));
    const guard = createSessionGuard({ userId: sessionIdentityKey(member), loadUserId, ...events });
    await guard.verify(); events.hide.mockClear(); events.show.mockClear();
    await guard.verify();
    expect(events.hide).toHaveBeenCalledOnce();
    expect(events.show).not.toHaveBeenCalled();
    expect(events.onInvalid).toHaveBeenCalledOnce();
  });
});
