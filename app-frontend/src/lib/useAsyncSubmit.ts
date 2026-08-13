// The submit ritual shared by every mutating button/form: one in-flight
// guard, errors normalized to String(e) (the IPC surface throws strings,
// not Errors), the previous error cleared on retry. The drifted copies
// (busy reset only on error, only in finally, or never) collapse to one
// policy: busy always clears when the call settles — close-on-success
// dialogs unmount before the extra render can re-enable anything.

import { useCallback, useRef, useState } from "react";

export interface AsyncSubmit {
  /** True from `run` entry until the call settles. */
  busy: boolean;
  /** Last failure, `String(e)`-normalized; cleared on the next `run`. */
  error: string | null;
  /** For surfaces that co-own the error line (e.g. a sibling fetch). */
  setError: (error: string | null) => void;
  /** Runs `fn` under the guard; re-entry while busy is a no-op (resolves
   * false). `onSuccess` runs after `fn` resolves — the close/clear tail
   * the try block used to hold. Resolves true on success. */
  run: (
    fn: () => Promise<unknown>,
    opts?: { onSuccess?: () => void },
  ) => Promise<boolean>;
}

export function useAsyncSubmit(): AsyncSubmit {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // The guard is a ref, not state: two clicks in one tick both render
  // busy=false, but only the first may pass.
  const running = useRef(false);

  const run = useCallback(
    async (fn: () => Promise<unknown>, opts?: { onSuccess?: () => void }) => {
      if (running.current) return false;
      running.current = true;
      setBusy(true);
      setError(null);
      try {
        await fn();
        opts?.onSuccess?.();
        return true;
      } catch (e) {
        setError(String(e));
        return false;
      } finally {
        running.current = false;
        setBusy(false);
      }
    },
    [],
  );

  return { busy, error, setError, run };
}
