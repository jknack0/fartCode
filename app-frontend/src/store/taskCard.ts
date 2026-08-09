// The task view's board cards (ADR-0037): the project's issue list, cached
// so the task header can resolve the open task's card, plus the pipeline
// overlay's state.
//
// Why a store and not component state: BoardView keeps its issues in
// component state, and the board is UNMOUNTED whenever the task view is up
// — which is exactly when the header needs the list. It is also the reason
// the header's actions can be registry commands at all: a command runs
// outside React, so everything it needs (card, columns, step flags, the
// overlay) has to be readable from a store.
//
// Load-and-cache like store/columns.ts: `load` is idempotent, `reload`
// forces a refetch, and the same events the board reloads on
// (issue:created/updated/deleted, step:launch, step:settled) refresh it
// here — every one of them can move the card between columns.

import { create } from "zustand";
import {
  issueList,
  onFartcodeEvent,
  type FartcodeEvent,
  type IssueDto,
} from "../lib/tauri";

/** Which pipeline overlay the task header is showing. `move` is the
 * key-first column picker; `confirm` names the spend of a parked step
 * before it fires. */
export type PipelineOverlay = "move" | "confirm";

interface TaskCardState {
  issuesByProject: Record<string, IssueDto[]>;
  loading: Record<string, boolean>;
  loaded: Record<string, boolean>;
  overlay: PipelineOverlay | null;
  /** Last pipeline action that failed. A move the engine refused must not
   * fail silently — the card would sit looking moved with nothing run. */
  error: string | null;
  load: (projectId: string) => Promise<void>;
  reload: (projectId: string) => Promise<void>;
  setOverlay: (overlay: PipelineOverlay | null) => void;
  setError: (error: string | null) => void;
}

// One process-wide subscription, wired lazily on first load (the columns
// and scripts stores use the same pattern — the app shell stays untouched).
let wired = false;

function wireEvents(): void {
  if (wired) return;
  wired = true;
  void onFartcodeEvent((event: FartcodeEvent) => {
    if (event.type === "project:deleted") {
      useTaskCard.setState((s) => {
        const issuesByProject = { ...s.issuesByProject };
        const loading = { ...s.loading };
        const loaded = { ...s.loaded };
        delete issuesByProject[event.id];
        delete loading[event.id];
        delete loaded[event.id];
        return { issuesByProject, loading, loaded };
      });
      return;
    }
    const projectId =
      event.type === "issue:created" ||
      event.type === "issue:updated" ||
      event.type === "issue:deleted" ||
      event.type === "step:launch" ||
      event.type === "step:settled"
        ? event.projectId
        : null;
    // Only refetch a project already in the cache: an event for a project
    // nobody is looking at must not pull its whole board over the bridge.
    if (projectId && useTaskCard.getState().loaded[projectId]) {
      void fetchInto(projectId);
    }
  });
}

async function fetchInto(projectId: string): Promise<void> {
  useTaskCard.setState((s) => ({ loading: { ...s.loading, [projectId]: true } }));
  try {
    const issues = await issueList(projectId);
    useTaskCard.setState((s) => ({
      issuesByProject: { ...s.issuesByProject, [projectId]: issues },
      loading: { ...s.loading, [projectId]: false },
      loaded: { ...s.loaded, [projectId]: true },
    }));
  } catch {
    // A board that cannot be read leaves the header with no pipeline
    // crumb and no pipeline actions, which is the honest rendering of
    // "we don't know where this card is" — not an error banner over a
    // task view whose agent is working fine.
    useTaskCard.setState((s) => ({ loading: { ...s.loading, [projectId]: false } }));
  }
}

export const useTaskCard = create<TaskCardState>((set, get) => ({
  issuesByProject: {},
  loading: {},
  loaded: {},
  overlay: null,
  error: null,

  load: async (projectId) => {
    wireEvents();
    const s = get();
    if (s.loading[projectId] || s.loaded[projectId]) return;
    await fetchInto(projectId);
  },

  reload: async (projectId) => {
    wireEvents();
    await fetchInto(projectId);
  },

  setOverlay: (overlay) => set({ overlay }),
  setError: (error) => set({ error }),
}));

/** Cards for a project — `[]` both before the first load and on an empty
 * board (the header renders the same either way: no pipeline crumb). */
export function cardsFor(projectId: string | null): IssueDto[] {
  return projectId ? (useTaskCard.getState().issuesByProject[projectId] ?? []) : [];
}
