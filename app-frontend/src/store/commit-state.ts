// Commit-card state store (E4-06): per-workspace repo state (branch, push
// remote, published, PR-open guard) driving the card's disabled matrix.
// Refetched on the same `git:changed`/`files:changed` debounce as the
// changes snapshot (wireCommitStateEvents, called from changes.ts) and
// immediately after commit/push so push/publish affordances flip without
// waiting for the watcher.
import { create } from "zustand";
import {
  gitCommit,
  gitCommitState,
  gitPush,
  type GitCommitResultDto,
  type GitCommitStateDto,
  type GitPushOutcomeDto,
} from "../lib/tauri";

export interface WorkspaceCommitState {
  state: GitCommitStateDto | null;
  error: string | null;
}

interface CommitStateStore {
  byWorkspace: Record<string, WorkspaceCommitState>;
  ensure: (workspaceId: string) => Promise<void>;
  refetch: (workspaceId: string) => Promise<void>;
  /** Commit the staged set; resolves the new hash. Throws on git errors. */
  commit: (workspaceId: string, message: string) => Promise<GitCommitResultDto>;
  /** Push current branch (set-upstream on first push). Throws on errors. */
  push: (workspaceId: string) => Promise<GitPushOutcomeDto>;
}

const inflight = new Set<string>();
const EMPTY: WorkspaceCommitState = { state: null, error: null };

export const useCommitState = create<CommitStateStore>((set, get) => {
  const patch = (workspaceId: string, part: Partial<WorkspaceCommitState>) =>
    set((s) => ({
      byWorkspace: {
        ...s.byWorkspace,
        [workspaceId]: { ...(s.byWorkspace[workspaceId] ?? EMPTY), ...part },
      },
    }));

  const fetchState = async (workspaceId: string) => {
    if (inflight.has(workspaceId)) return;
    inflight.add(workspaceId);
    try {
      const state = await gitCommitState(workspaceId);
      patch(workspaceId, { state, error: null });
    } catch (e) {
      patch(workspaceId, { error: String(e) });
    } finally {
      inflight.delete(workspaceId);
    }
  };

  return {
    byWorkspace: {},

    ensure: async (workspaceId) => {
      const entry = get().byWorkspace[workspaceId];
      if (entry?.state || inflight.has(workspaceId)) return;
      await fetchState(workspaceId);
    },

    refetch: async (workspaceId) => {
      await fetchState(workspaceId);
    },

    commit: async (workspaceId, message) => {
      const result = await gitCommit(workspaceId, message);
      await fetchState(workspaceId);
      return result;
    },

    push: async (workspaceId) => {
      const outcome = await gitPush(workspaceId);
      await fetchState(workspaceId);
      return outcome;
    },
  };
});

/** Test seam (browser smoke), mirrors `window.__changesStore`. */
declare global {
  interface Window {
    __commitStateStore?: typeof useCommitState;
  }
}
if (typeof window !== "undefined") window.__commitStateStore = useCommitState;
