// Open-file intent (E5-01 → E5-02 seam): the file tree emits, the editor
// tabs (E5-02) will subscribe. Until then clicks are a visible no-op — the
// seam exists so E5-02 is a pure consumer change.
export interface OpenFileIntent {
  taskId: string;
  workspaceId: string;
  /** Worktree-relative path. */
  path: string;
}

type Listener = (intent: OpenFileIntent) => void;
const listeners = new Set<Listener>();

export function onOpenFile(cb: Listener): () => void {
  listeners.add(cb);
  return () => listeners.delete(cb);
}

export function emitOpenFile(intent: OpenFileIntent): void {
  for (const cb of listeners) cb(intent);
}
