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
import { createKeyedCache } from "../lib/createKeyedStore";
import { wireEvents } from "../lib/wireEvents";

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
  /** Tabs whose disk content changed while the editor was dirty (#130 —
   * the "changed on disk" badge; reload-vs-keep resolves it). */
  externalByTab: Record<string, true>;
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
  markExternal: (tabId: string) => void;
  clearExternal: (tabId: string) => void;
  /** ⌘S: write the editor's worktree-side document to disk. The refresh
   * after the write arrives via the watcher's files:changed event. */
  save: (tabId: string) => Promise<void>;
  setSelection: (tabId: string, selection: DiffSelection | null) => void;
  ensureMode: () => Promise<void>;
  setMode: (mode: DiffMode) => void;
}

const EMPTY: DiffEntry = { payload: null, loading: false, error: null };
const pendingRefresh = new Map<string, number>();

export const useDiffs = create<DiffsState>((set, get) => {
  const cache = createKeyedCache<DiffEntry, FileDiffDto>({
    empty: EMPTY,
    read: () => get().byTab,
    write: (byTab) => set({ byTab }),
    success: (payload) => ({ payload, loading: false, error: null }),
    failure: (error) => ({ loading: false, error }),
  });

  const fetchPayload = (tabId: string, params: DiffParams) =>
    cache.run(tabId, () =>
      gitFileDiff(params.workspaceId, params.path, params.side, params.origPath),
    );

  return {
    paramsByTab: {},
    previewTabs: {},
    byTab: {},
    dirtyByTab: {},
    saveErrorByTab: {},
    externalByTab: {},
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
        const externalByTab = { ...s.externalByTab };
        const selectionByTab = { ...s.selectionByTab };
        delete paramsByTab[tabId];
        delete previewTabs[tabId];
        delete byTab[tabId];
        delete dirtyByTab[tabId];
        delete saveErrorByTab[tabId];
        delete externalByTab[tabId];
        delete selectionByTab[tabId];
        return {
          paramsByTab,
          previewTabs,
          byTab,
          dirtyByTab,
          saveErrorByTab,
          externalByTab,
          selectionByTab,
        };
      }),

    ensure: async (tabId, params) => {
      const entry = get().byTab[tabId];
      if (entry?.payload || cache.inflight(tabId)) return;
      cache.patch(tabId, { loading: true });
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

    markExternal: (tabId) =>
      set((s) =>
        s.externalByTab[tabId] ? s : { externalByTab: { ...s.externalByTab, [tabId]: true } },
      ),

    clearExternal: (tabId) =>
      set((s) => {
        if (!s.externalByTab[tabId]) return s;
        const externalByTab = { ...s.externalByTab };
        delete externalByTab[tabId];
        return { externalByTab };
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
        // Saving overwrites the disk version deliberately — any pending
        // divergence badge is resolved (#130).
        get().clearExternal(tabId);
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
  const unwire = wireEvents(onFartcodeEvent, (event) => {
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
  });
  return () => {
    unwire();
    for (const timer of pendingRefresh.values()) clearTimeout(timer);
    pendingRefresh.clear();
  };
}
