// Store event wiring. Tauri's `listen` resolves its unlisten fn from a
// promise, which opens a race every hand-rolled wiring had to close
// itself: cleanup can run BEFORE the promise resolves, so `unlisten` is
// still null and the subscription leaks — permanently, under
// React.StrictMode, where the first mount's cleanup always loses the
// race. The disposed guard lives here once; wirings become one-liners.

type Unlisten = () => void;

/**
 * Subscribes `handler` through a promise-based `listen` and returns the
 * dispose fn. Dispose is safe at any time: before the listen promise
 * resolves it marks the wiring dead (the subscription detaches the moment
 * the promise lands), after it detaches immediately. A handler never
 * fires after dispose.
 */
export function wireEvents<T>(
  listen: (handler: (payload: T) => void) => Promise<Unlisten>,
  handler: (payload: T) => void,
): Unlisten {
  let unlisten: Unlisten | null = null;
  let disposed = false;
  void listen((payload) => {
    if (!disposed) handler(payload);
  }).then((off) => {
    if (disposed) off();
    else unlisten = off;
  });
  return () => {
    disposed = true;
    unlisten?.();
    unlisten = null;
  };
}
