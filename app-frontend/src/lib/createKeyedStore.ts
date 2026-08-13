// Per-key async-cache primitives for the keyed stores (changes,
// commit-state, pr, diffs, editors — one payload cached per workspace or
// tab id). The contract used to live copy-pasted in every store: a
// module-level in-flight Set deduping concurrent fetches, a spread
// `patch` into the entry record, and the fetch envelope writing
// `success(result)` on resolve vs `failure(String(e))` on reject. Policy
// changes (error normalization, retry, eviction) now land here once.
//
// Deliberately NOT a whole zustand store: each store keeps its named
// record field (`byWorkspace` / `byTab`), its entry field names
// (`snapshot` / `section` / ...) that components already read, and its
// domain actions — it composes these primitives inside `create`.

export interface KeyedCache<E, T> {
  /** True while `key` has a fetch in flight (the ensure gate). */
  inflight: (key: string) => boolean;
  /** Spread-merges `part` into the entry (absent entries start `empty`). */
  patch: (key: string, part: Partial<E>) => void;
  /** The fetch envelope: dedupes on `key`, then writes `success(result)`
   * or `failure(String(e))` into the entry. `thunk` carries any fetch
   * params — the cache never knows how a payload is fetched. */
  run: (key: string, thunk: () => Promise<T>) => Promise<void>;
}

export function createKeyedCache<E, T>(opts: {
  /** Entry defaults for a key never seen before. */
  empty: E;
  /** Read the current record (the store's `get`). */
  read: () => Record<string, E>;
  /** Write the record back (the store's `set`, under its field name). */
  write: (byKey: Record<string, E>) => void;
  /** Entry patch for a resolved fetch. */
  success: (result: T) => Partial<E>;
  /** Entry patch for a failed fetch (`message` is `String(e)`). */
  failure: (message: string) => Partial<E>;
}): KeyedCache<E, T> {
  const { empty, read, write, success, failure } = opts;
  const inflight = new Set<string>();

  const patch = (key: string, part: Partial<E>) => {
    const byKey = read();
    write({ ...byKey, [key]: { ...(byKey[key] ?? empty), ...part } });
  };

  return {
    inflight: (key) => inflight.has(key),
    patch,
    run: async (key, thunk) => {
      if (inflight.has(key)) return;
      inflight.add(key);
      try {
        patch(key, success(await thunk()));
      } catch (e) {
        patch(key, failure(String(e)));
      } finally {
        inflight.delete(key);
      }
    },
  };
}
