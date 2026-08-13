// wireEvents: pins the disposed-race contract — cleanup that runs before
// the listen promise resolves still detaches the subscription (the
// StrictMode leak every hand-rolled wiring had to guard against), and no
// handler fires after dispose.
import { describe, it, expect, vi } from "vitest";
import { wireEvents } from "./wireEvents";

describe("wireEvents", () => {
  it("delivers payloads, then detaches on dispose", async () => {
    let emit!: (p: string) => void;
    const off = vi.fn();
    const listen = (h: (p: string) => void) => {
      emit = h;
      return Promise.resolve(off);
    };
    const seen: string[] = [];
    const dispose = wireEvents(listen, (p) => seen.push(p));
    await Promise.resolve();
    emit("a");
    dispose();
    // An event already in flight at dispose time must not land.
    emit("b");
    expect(seen).toEqual(["a"]);
    expect(off).toHaveBeenCalledTimes(1);
  });

  it("detaches even when dispose loses the race to listen", async () => {
    const off = vi.fn();
    let resolveListen!: (u: () => void) => void;
    const listen = () => new Promise<() => void>((res) => (resolveListen = res));
    const dispose = wireEvents(listen, () => {});
    dispose(); // cleanup runs first — the hand-rolled copies leaked here
    resolveListen(off);
    await Promise.resolve();
    expect(off).toHaveBeenCalledTimes(1);
  });
});
