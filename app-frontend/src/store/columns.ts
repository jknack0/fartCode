// Board columns (E18-07, ADR-0037): the project's pipeline config, cached
// per project and read by the board (§8a headers, N-column grid, h/l walk)
// and — next round — the settings Columns editor (#67).
//
// Load-and-cache, like the dependency store: `load(projectId)` is cheap and
// idempotent, `reload` forces a refetch. There is deliberately NO
// `column:*` backend event yet — column config only changes through this
// frontend's own CRUD calls — so mutators must call `reload` after writing;
// the store drops a project's cache on `project:deleted` so a re-added path
// never serves stale columns.

import { create } from "zustand";
import {
  columnList,
  onFartcodeEvent,
  type BoardColumnDto,
  type FartcodeEvent,
} from "../lib/tauri";
import { sortColumns } from "../lib/columnConfig";

interface ColumnsState {
  /** Columns in board order (position), by project id. */
  byProject: Record<string, BoardColumnDto[]>;
  loading: Record<string, boolean>;
  /** Project ids whose first load has resolved — an empty array before the
   * first load means "not read yet", not "no columns". */
  loaded: Record<string, boolean>;
  error: string | null;
  /** Fetches once per project; a second call is a no-op while in flight. */
  load: (projectId: string) => Promise<void>;
  /** Refetches regardless of cache — call after any column mutation. */
  reload: (projectId: string) => Promise<void>;
}

// One process-wide subscription, wired lazily on first load so the app
// shell stays untouched (the scripts store uses the same pattern).
let evictWired = false;

function wireEviction(): void {
  if (evictWired) return;
  evictWired = true;
  void onFartcodeEvent((event: FartcodeEvent) => {
    if (event.type !== "project:deleted") return;
    useColumns.setState((s) => {
      const byProject = { ...s.byProject };
      const loading = { ...s.loading };
      const loaded = { ...s.loaded };
      delete byProject[event.id];
      delete loading[event.id];
      delete loaded[event.id];
      return { byProject, loading, loaded };
    });
  });
}

async function fetchInto(projectId: string): Promise<void> {
  useColumns.setState((s) => ({
    loading: { ...s.loading, [projectId]: true },
    error: null,
  }));
  try {
    const columns = sortColumns(await columnList(projectId));
    useColumns.setState((s) => ({
      byProject: { ...s.byProject, [projectId]: columns },
      loading: { ...s.loading, [projectId]: false },
      loaded: { ...s.loaded, [projectId]: true },
    }));
  } catch (e) {
    useColumns.setState((s) => ({
      error: String(e),
      loading: { ...s.loading, [projectId]: false },
    }));
  }
}

export const useColumns = create<ColumnsState>((set, get) => {
  void set;
  return {
    byProject: {},
    loading: {},
    loaded: {},
    error: null,

    load: async (projectId) => {
      wireEviction();
      const s = get();
      if (s.loading[projectId] || s.loaded[projectId]) return;
      await fetchInto(projectId);
    },

    reload: async (projectId) => {
      wireEviction();
      await fetchInto(projectId);
    },
  };
});

/** Columns for a project in board order — `[]` both before the first load
 * and on a project with none (check `loaded` when the difference matters). */
export function columnsFor(projectId: string | null): BoardColumnDto[] {
  return projectId ? (useColumns.getState().byProject[projectId] ?? []) : [];
}
