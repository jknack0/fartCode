// createKeyedCache: pins the envelope contract the keyed stores share —
// in-flight dedupe per key, success/failure patches spread onto the entry
// (absent entries start from `empty`), errors String(e)-normalized.
import { describe, it, expect, vi } from "vitest";
import { createKeyedCache } from "./createKeyedStore";

interface Entry {
  payload: string | null;
  loading: boolean;
  error: string | null;
}

const EMPTY: Entry = { payload: null, loading: false, error: null };

function makeCache() {
  let byKey: Record<string, Entry> = {};
  const cache = createKeyedCache<Entry, string>({
    empty: EMPTY,
    read: () => byKey,
    write: (next) => (byKey = next),
    success: (payload) => ({ payload, loading: false, error: null }),
    failure: (error) => ({ loading: false, error }),
  });
  return { cache, get: () => byKey };
}

describe("createKeyedCache", () => {
  it("dedupes concurrent runs per key and writes the success patch", async () => {
    const { cache, get } = makeCache();
    let resolveFetch!: (v: string) => void;
    const fetcher = vi.fn(
      () => new Promise<string>((res) => (resolveFetch = res)),
    );
    const first = cache.run("k", fetcher);
    expect(cache.inflight("k")).toBe(true);
    await cache.run("k", fetcher); // deduped — resolves without fetching
    expect(fetcher).toHaveBeenCalledTimes(1);
    resolveFetch("v");
    await first;
    expect(cache.inflight("k")).toBe(false);
    expect(get().k).toEqual({ payload: "v", loading: false, error: null });
  });

  it("normalizes failures and preserves the rest of the entry", async () => {
    const { cache, get } = makeCache();
    cache.patch("k", { payload: "stale" });
    await cache.run("k", () => Promise.reject("boom"));
    // The failure patch merges — the stale payload stays on screen.
    expect(get().k).toEqual({ payload: "stale", loading: false, error: "boom" });
    expect(cache.inflight("k")).toBe(false);
  });
});
