// Changes store (E4-03): per-workspace git status snapshots, refreshed on
// `git:changed`/`files:changed` bus events — never polled. Row actions
// (stage/unstage/discard) invoke the command and refetch immediately so the
// UI feels synchronous; the watcher's event then confirms the same state.
import { create } from "zustand";
import { useCommitState } from "./commit-state";
import {
  gitDiscard,
  gitStage,
  gitStageAll,
  gitStatus,
  gitUnstage,
  onFartcodeEvent,
  type GitStatusSnapshot,
} from "../lib/tauri";
import { createKeyedCache } from "../lib/createKeyedStore";
import { wireEvents } from "../lib/wireEvents";

export interface WorkspaceChanges {
  snapshot: GitStatusSnapshot | null;
  /** True while the first fetch is in flight (later refetches are quiet —
   * the snapshot stays on screen). */
  loading: boolean;
  error: string | null;
}

interface ChangesState {
  byWorkspace: Record<string, WorkspaceChanges>;
  ensure: (workspaceId: string) => Promise<void>;
  refetch: (workspaceId: string) => Promise<void>;
  stage: (workspaceId: string, paths: string[]) => Promise<void>;
  stageAll: (workspaceId: string) => Promise<void>;
  unstage: (workspaceId: string, paths: string[]) => Promise<void>;
  discard: (workspaceId: string, paths: string[]) => Promise<void>;
}

/** Per-workspace debounce timer for event-driven refetches (DOM handles). */
const pendingRefetch = new Map<string, number>();

const EMPTY: WorkspaceChanges = { snapshot: null, loading: false, error: null };

export const useChanges = create<ChangesState>((set, get) => {
  const cache = createKeyedCache<WorkspaceChanges, GitStatusSnapshot>({
    empty: EMPTY,
    read: () => get().byWorkspace,
    write: (byWorkspace) => set({ byWorkspace }),
    success: (snapshot) => ({ snapshot, loading: false, error: null }),
    failure: (error) => ({ loading: false, error }),
  });

  const fetchSnapshot = (workspaceId: string) =>
    cache.run(workspaceId, () => gitStatus(workspaceId));

  return {
    byWorkspace: {},

    ensure: async (workspaceId) => {
      const entry = get().byWorkspace[workspaceId];
      if (entry?.snapshot || cache.inflight(workspaceId)) return;
      cache.patch(workspaceId, { loading: true });
      await fetchSnapshot(workspaceId);
    },

    refetch: async (workspaceId) => {
      await fetchSnapshot(workspaceId);
    },

    stage: async (workspaceId, paths) => {
      await gitStage(workspaceId, paths);
      await fetchSnapshot(workspaceId);
    },

    stageAll: async (workspaceId) => {
      await gitStageAll(workspaceId);
      await fetchSnapshot(workspaceId);
    },

    unstage: async (workspaceId, paths) => {
      await gitUnstage(workspaceId, paths);
      await fetchSnapshot(workspaceId);
    },

    discard: async (workspaceId, paths) => {
      await gitDiscard(workspaceId, paths);
      await fetchSnapshot(workspaceId);
    },
  };
});

/** Test seam (browser smoke), mirrors `window.__conversationsStore`. */
declare global {
  interface Window {
    __changesStore?: typeof useChanges;
  }
}
if (typeof window !== "undefined") window.__changesStore = useChanges;

/** Coalesced event-driven refresh: a burst of git/fs events collapses into
 * one refetch per tracked workspace ~150ms after the last event. Only
 * workspaces the UI already tracks are refreshed (nothing is fetched for a
 * workspace no panel has opened). */
export function wireChangesEvents(): () => void {
  const unwire = wireEvents(onFartcodeEvent, (event) => {
    const workspaceId =
      event.type === "git:changed" || event.type === "files:changed"
        ? event.workspaceId
        : null;
    if (!workspaceId) return;
    if (!(workspaceId in useChanges.getState().byWorkspace)) return;
    clearTimeout(pendingRefetch.get(workspaceId));
    pendingRefetch.set(
      workspaceId,
      setTimeout(() => {
        pendingRefetch.delete(workspaceId);
        void useChanges.getState().refetch(workspaceId);
        // E4-06: the commit card's repo state rides the same debounce.
        if (workspaceId in useCommitState.getState().byWorkspace) {
          void useCommitState.getState().refetch(workspaceId);
        }
      }, 150),
    );
  });
  return () => {
    unwire();
    for (const timer of pendingRefetch.values()) clearTimeout(timer);
    pendingRefetch.clear();
  };
}
