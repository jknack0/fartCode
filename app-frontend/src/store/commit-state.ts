// Commit-card state store (E4-06): per-workspace repo state (branch, push
// remote, published, PR-open guard) driving the card's disabled matrix.
// Refetched on the same `git:changed`/`files:changed` debounce as the
// changes snapshot (wireCommitStateEvents, called from changes.ts) and
// immediately after commit/push so push/publish affordances flip without
// waiting for the watcher.
import { create } from "zustand";
import {
  gitAddRemote,
  gitCommit,
  gitCommitState,
  gitFetch,
  gitPublish,
  gitPull,
  gitPush,
  type GitCommitResultDto,
  type GitCommitStateDto,
  type GitPublishOutcomeDto,
  type GitPushOutcomeDto,
} from "../lib/tauri";
import { createKeyedCache } from "../lib/createKeyedStore";

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
  // -- E4-08 footer verbs (each refetches state so the footer flips) ----
  fetch: (workspaceId: string) => Promise<void>;
  pull: (workspaceId: string) => Promise<void>;
  publish: (workspaceId: string) => Promise<GitPublishOutcomeDto>;
  addRemote: (workspaceId: string, name: string, url: string) => Promise<void>;
}

const EMPTY: WorkspaceCommitState = { state: null, error: null };

export const useCommitState = create<CommitStateStore>((set, get) => {
  const cache = createKeyedCache<WorkspaceCommitState, GitCommitStateDto>({
    empty: EMPTY,
    read: () => get().byWorkspace,
    write: (byWorkspace) => set({ byWorkspace }),
    success: (state) => ({ state, error: null }),
    failure: (error) => ({ error }),
  });

  const fetchState = (workspaceId: string) =>
    cache.run(workspaceId, () => gitCommitState(workspaceId));

  return {
    byWorkspace: {},

    ensure: async (workspaceId) => {
      const entry = get().byWorkspace[workspaceId];
      if (entry?.state || cache.inflight(workspaceId)) return;
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

    fetch: async (workspaceId) => {
      await gitFetch(workspaceId);
      await fetchState(workspaceId);
    },

    pull: async (workspaceId) => {
      await gitPull(workspaceId);
      await fetchState(workspaceId);
    },

    publish: async (workspaceId) => {
      const outcome = await gitPublish(workspaceId);
      await fetchState(workspaceId);
      return outcome;
    },

    addRemote: async (workspaceId, name, url) => {
      await gitAddRemote(workspaceId, name, url);
      await fetchState(workspaceId);
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
