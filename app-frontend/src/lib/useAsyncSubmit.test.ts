// useAsyncSubmit: pins the submit-ritual policy — one in-flight guard
// (re-entry resolves false without calling fn), String(e) normalization,
// error cleared on retry, busy always reset when the call settles.
import { describe, it, expect, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { useAsyncSubmit } from "./useAsyncSubmit";

describe("useAsyncSubmit", () => {
  it("guards re-entry while busy and resets busy on settle", async () => {
    const { result } = renderHook(() => useAsyncSubmit());
    let release!: () => void;
    const fn = vi.fn(() => new Promise<void>((res) => (release = res)));

    let first!: Promise<boolean>;
    act(() => {
      first = result.current.run(fn);
    });
    expect(result.current.busy).toBe(true);

    let second!: Promise<boolean>;
    act(() => {
      second = result.current.run(fn);
    });
    await expect(second).resolves.toBe(false);
    expect(fn).toHaveBeenCalledTimes(1);

    await act(async () => {
      release();
      await expect(first).resolves.toBe(true);
    });
    expect(result.current.busy).toBe(false);
  });

  it("normalizes errors, clears them on retry, runs onSuccess", async () => {
    const { result } = renderHook(() => useAsyncSubmit());
    await act(async () => {
      await expect(
        result.current.run(() => Promise.reject("boom")),
      ).resolves.toBe(false);
    });
    expect(result.current.error).toBe("boom");
    expect(result.current.busy).toBe(false);

    const onSuccess = vi.fn();
    await act(async () => {
      await result.current.run(() => Promise.resolve(), { onSuccess });
    });
    expect(result.current.error).toBeNull();
    expect(onSuccess).toHaveBeenCalledTimes(1);
  });
});
