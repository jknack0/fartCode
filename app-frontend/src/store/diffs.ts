// Diffs store (E4-04): per-tab diff payloads + preview bookkeeping for the
// diff tab kind. Params live here keyed by tab id (restored tabs re-parse
// their id instead — see lib/diff-tabs.ts). Preview tabs are a
// per-pane-of-one: opening a preview replaces the pane's current preview;
// restored tabs have no preview record and behave as persistent.
import { create } from "zustand";
import {
  getViewState,
  gitFileDiff,
  onFartcodeEvent,
  setViewState,
  writeWorkspaceFile,
  type DiffSide,
  type FileDiffDto,
} from "../lib/tauri";
import { diffViewWorktreeDoc, getDiffView } from "../lib/diff-views";

export interface DiffParams {
  workspaceId: string;
  path: string;
  origPath: string | null;
  side: DiffSide;
}

export interface DiffEntry {
  payload: FileDiffDto | null;
  loading: boolean;
  error: string | null;
}

export type DiffMode = "unified" | "split";

/** A text selection inside a diff editor (E4 selection → agent). Side `a`
 * is the baseline, `b` the worktree, `single` the one-document views. */
export interface DiffSelection {
  side: "a" | "b" | "single";
  from: number;
  to: number;
  fromLine: number;
  toLine: number;
  text: string;
}

const DIFF_MODE_KEY = "view-state:app:diff-mode";

interface DiffsState {
  paramsByTab: Record<string, DiffParams>;
  previewTabs: Record<string, true>;
  byTab: Record<string, DiffEntry>;
  /** Tabs with unsaved editor changes (E4-05 — the dirty dot). */
  dirtyByTab: Record<string, true>;
  /** Last save failure per tab (rendered as a header chip). */
  saveErrorByTab: Record<string, string>;
  /** Live text selection per tab (null when collapsed). */
  selectionByTab: Record<string, DiffSelection | null>;
  mode: DiffMode;
  modeLoaded: boolean;

  setParams: (tabId: string, params: DiffParams) => void;
  setPreview: (tabId: string, preview: boolean) => void;
  dropTab: (tabId: string) => void;
  ensure: (tabId: string, params: DiffParams) => Promise<void>;
  refresh: (tabId: string) => Promise<void>;
  markDirty: (tabId: string) => void;
  clearDirty: (tabId: string) => void;
  /** ⌘S: write the editor's worktree-side document to disk. The refresh
   * after the write arrives via the watcher's files:changed event. */
  save: (tabId: string) => Promise<void>;
  setSelection: (tabId: string, selection: DiffSelection | null) => void;
  ensureMode: () => Promise<void>;
  setMode: (mode: DiffMode) => void;
}

const EMPTY: DiffEntry = { payload: null, loading: false, error: null };
const inflight = new Set<string>();
const pendingRefresh = new Map<string, number>();

export const useDiffs = create<DiffsState>((set, get) => {
  const patch = (tabId: string, part: Partial<DiffEntry>) =>
    set((s) => ({
      byTab: { ...s.byTab, [tabId]: { ...(s.byTab[tabId] ?? EMPTY), ...part } },
    }));

  const fetchPayload = async (tabId: string, params: DiffParams) => {
    if (inflight.has(tabId)) return;
    inflight.add(tabId);
    try {
      const payload = await gitFileDiff(
        params.workspaceId,
        params.path,
        params.side,
        params.origPath,
      );
      patch(tabId, { payload, loading: false, error: null });
    } catch (e) {
      patch(tabId, { loading: false, error: String(e) });
    } finally {
      inflight.delete(tabId);
    }
  };

  return {
    paramsByTab: {},
    previewTabs: {},
    byTab: {},
    dirtyByTab: {},
    saveErrorByTab: {},
    selectionByTab: {},
    mode: "split",
    modeLoaded: false,

    setParams: (tabId, params) =>
      set((s) => ({ paramsByTab: { ...s.paramsByTab, [tabId]: params } })),

    setPreview: (tabId, preview) =>
      set((s) => {
        const previewTabs = { ...s.previewTabs };
        if (preview) previewTabs[tabId] = true;
        else delete previewTabs[tabId];
        return { previewTabs };
      }),

    dropTab: (tabId) =>
      set((s) => {
        const paramsByTab = { ...s.paramsByTab };
        const previewTabs = { ...s.previewTabs };
        const byTab = { ...s.byTab };
        const dirtyByTab = { ...s.dirtyByTab };
        const saveErrorByTab = { ...s.saveErrorByTab };
        const selectionByTab = { ...s.selectionByTab };
        delete paramsByTab[tabId];
        delete previewTabs[tabId];
        delete byTab[tabId];
        delete dirtyByTab[tabId];
        delete saveErrorByTab[tabId];
        delete selectionByTab[tabId];
        return { paramsByTab, previewTabs, byTab, dirtyByTab, saveErrorByTab, selectionByTab };
      }),

    ensure: async (tabId, params) => {
      const entry = get().byTab[tabId];
      if (entry?.payload || inflight.has(tabId)) return;
      patch(tabId, { loading: true });
      await fetchPayload(tabId, params);
    },

    refresh: async (tabId) => {
      const params = get().paramsByTab[tabId];
      if (!params) return;
      await fetchPayload(tabId, params);
    },

    markDirty: (tabId) =>
      set((s) =>
        s.dirtyByTab[tabId] ? s : { dirtyByTab: { ...s.dirtyByTab, [tabId]: true } },
      ),

    clearDirty: (tabId) =>
      set((s) => {
        if (!s.dirtyByTab[tabId]) return s;
        const dirtyByTab = { ...s.dirtyByTab };
        delete dirtyByTab[tabId];
        return { dirtyByTab };
      }),

    save: async (tabId) => {
      const params = get().paramsByTab[tabId];
      const view = getDiffView(tabId);
      if (!params || !view) return;
      try {
        await writeWorkspaceFile(
          params.workspaceId,
          params.path,
          diffViewWorktreeDoc(view),
        );
        get().clearDirty(tabId);
        set((s) => {
          if (!s.saveErrorByTab[tabId]) return s;
          const saveErrorByTab = { ...s.saveErrorByTab };
          delete saveErrorByTab[tabId];
          return { saveErrorByTab };
        });
        // The watcher's files:changed event drives the payload refresh —
        // the editor already shows the saved document, so the refresh is
        // a no-op visually (DiffView skips content-identical rebuilds).
      } catch (e) {
        set((s) => ({
          saveErrorByTab: { ...s.saveErrorByTab, [tabId]: String(e) },
        }));
      }
    },

    setSelection: (tabId, selection) =>
      set((s) => {
        const prev = s.selectionByTab[tabId] ?? null;
        if (prev === selection) return s;
        if (
          prev &&
          selection &&
          prev.side === selection.side &&
          prev.from === selection.from &&
          prev.to === selection.to
        ) {
          return s;
        }
        return { selectionByTab: { ...s.selectionByTab, [tabId]: selection } };
      }),

    ensureMode: async () => {
      if (get().modeLoaded) return;
      set({ modeLoaded: true });
      try {
        const saved = await getViewState(DIFF_MODE_KEY);
        if (saved === "unified" || saved === "split") set({ mode: saved });
      } catch {
        /* view-state read failure keeps the default */
      }
    },

    setMode: (mode) => {
      set({ mode });
      void setViewState(DIFF_MODE_KEY, mode).catch(() => {});
    },
  };
});

/** Coalesced refresh of open diff tabs when their workspace changes. */
export function wireDiffsEvents(): () => void {
  let unlisten: (() => void) | null = null;
  let disposed = false;
  void onFartcodeEvent((event) => {
    const workspaceId =
      event.type === "git:changed" || event.type === "files:changed"
        ? event.workspaceId
        : null;
    if (!workspaceId) return;
    const { byTab, paramsByTab } = useDiffs.getState();
    const tabIds = Object.keys(byTab).filter(
      (id) => paramsByTab[id]?.workspaceId === workspaceId,
    );
    if (tabIds.length === 0) return;
    clearTimeout(pendingRefresh.get(workspaceId));
    pendingRefresh.set(
      workspaceId,
      setTimeout(() => {
        pendingRefresh.delete(workspaceId);
        for (const id of tabIds) void useDiffs.getState().refresh(id);
      }, 150),
    );
  }).then((off) => {
    if (disposed) off();
    else unlisten = off;
  });
  return () => {
    disposed = true;
    unlisten?.();
    for (const timer of pendingRefresh.values()) clearTimeout(timer);
    pendingRefresh.clear();
  };
}
