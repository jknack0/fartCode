// PR section store (E4-07/E4-09): per-workspace cached PR payloads read from
// the sync cache (`pr_section_get` is cache-first + background sync). Live
// updates arrive as `pr:updated` events; `git:changed` re-syncs debounced so
// commit/push head changes converge. The UI never polls — the backend
// scheduler is the only timer.
import { create } from "zustand";
import {
  onFartcodeEvent,
  prSectionGet,
  prSectionSync,
  type PrSectionDto,
} from "../lib/tauri";
import { createKeyedCache } from "../lib/createKeyedStore";
import { wireEvents } from "../lib/wireEvents";

export interface WorkspacePr {
  section: PrSectionDto | null;
  /** True while the first fetch is in flight. */
  loading: boolean;
  error: string | null;
}

interface PrState {
  byWorkspace: Record<string, WorkspacePr>;
  /** Cache read + background sync (tab open / event refresh). */
  ensure: (workspaceId: string) => Promise<void>;
  refetch: (workspaceId: string) => Promise<void>;
  /** Awaits one sync pass (explicit refresh button). */
  sync: (workspaceId: string) => Promise<void>;
}

const pendingRefetch = new Map<string, number>();
const EMPTY: WorkspacePr = { section: null, loading: false, error: null };

export const usePr = create<PrState>((set, get) => {
  const cache = createKeyedCache<WorkspacePr, PrSectionDto>({
    empty: EMPTY,
    read: () => get().byWorkspace,
    write: (byWorkspace) => set({ byWorkspace }),
    success: (section) => ({ section, loading: false, error: null }),
    failure: (error) => ({ loading: false, error }),
  });

  const fetchSection = (workspaceId: string, awaitedSync: boolean) =>
    cache.run(workspaceId, () =>
      awaitedSync ? prSectionSync(workspaceId) : prSectionGet(workspaceId),
    );

  return {
    byWorkspace: {},

    ensure: async (workspaceId) => {
      const entry = get().byWorkspace[workspaceId];
      if (entry?.section || cache.inflight(workspaceId)) return;
      cache.patch(workspaceId, { loading: true });
      await fetchSection(workspaceId, false);
    },

    refetch: async (workspaceId) => {
      await fetchSection(workspaceId, false);
    },

    sync: async (workspaceId) => {
      cache.patch(workspaceId, { loading: true });
      await fetchSection(workspaceId, true);
    },
  };
});

declare global {
  interface Window {
    __prStore?: typeof usePr;
  }
}
if (typeof window !== "undefined") window.__prStore = usePr;

/** Coalesced event refresh for tracked workspaces only. */
export function wirePrEvents(): () => void {
  const unwire = wireEvents(onFartcodeEvent, (event) => {
    let workspaceId: string | null = null;
    let immediate = false;
    if (event.type === "pr:updated") {
      workspaceId = event.workspaceId;
      immediate = true; // the sync engine already changed the cache
    } else if (event.type === "git:changed") {
      workspaceId = event.workspaceId;
    }
    if (!workspaceId) return;
    if (!(workspaceId in usePr.getState().byWorkspace)) return;
    clearTimeout(pendingRefetch.get(workspaceId));
    pendingRefetch.set(
      workspaceId,
      setTimeout(
        () => {
          pendingRefetch.delete(workspaceId);
          void usePr.getState().refetch(workspaceId);
        },
        immediate ? 50 : 750,
      ),
    );
  });
  return () => {
    unwire();
    for (const timer of pendingRefetch.values()) clearTimeout(timer);
    pendingRefetch.clear();
  };
}
