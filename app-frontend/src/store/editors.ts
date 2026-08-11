// Editors store (E5-02): per-tab file content + dirty/preview/save-error
// bookkeeping for the file-editor tab kind. The tab id encodes the params
// (`edit:<workspaceId>:<path>`) so restored tabs re-parse instead of
// persisting a sidecar — the diff store's exact shape. The live EditorView
// registry lives here too (module map, not state — views aren't
// serializable and never re-render anything).
import { create } from "zustand";
import type { EditorView } from "@codemirror/view";
import { readWorkspaceFile, writeWorkspaceFile } from "../lib/tauri";
import { parseEditorTabId } from "../lib/editor-tabs";

export interface EditorEntry {
  /** Document as loaded from disk (the editor owns live text). */
  content: string | null;
  loading: boolean;
  error: string | null;
}

const views = new Map<string, EditorView>();
export function registerEditorView(tabId: string, view: EditorView): void {
  views.set(tabId, view);
}
export function unregisterEditorView(tabId: string): void {
  views.delete(tabId);
}

interface EditorsState {
  byTab: Record<string, EditorEntry>;
  previewTabs: Record<string, true>;
  dirtyByTab: Record<string, true>;
  saveErrorByTab: Record<string, string>;

  setPreview: (tabId: string, preview: boolean) => void;
  dropTab: (tabId: string) => void;
  ensure: (tabId: string) => Promise<void>;
  markDirty: (tabId: string) => void;
  clearDirty: (tabId: string) => void;
  /** ⌘S: write the live editor document to disk. */
  save: (tabId: string) => Promise<void>;
  /** ⌘⇧S: save every dirty editor tab. */
  saveAll: () => Promise<void>;
}

const EMPTY: EditorEntry = { content: null, loading: false, error: null };
const inflight = new Set<string>();

export const useEditors = create<EditorsState>((set, get) => {
  const patch = (tabId: string, part: Partial<EditorEntry>) =>
    set((s) => ({
      byTab: { ...s.byTab, [tabId]: { ...(s.byTab[tabId] ?? EMPTY), ...part } },
    }));

  return {
    byTab: {},
    previewTabs: {},
    dirtyByTab: {},
    saveErrorByTab: {},

    setPreview: (tabId, preview) =>
      set((s) => {
        const previewTabs = { ...s.previewTabs };
        if (preview) previewTabs[tabId] = true;
        else delete previewTabs[tabId];
        return { previewTabs };
      }),

    dropTab: (tabId) =>
      set((s) => {
        const byTab = { ...s.byTab };
        const previewTabs = { ...s.previewTabs };
        const dirtyByTab = { ...s.dirtyByTab };
        const saveErrorByTab = { ...s.saveErrorByTab };
        delete byTab[tabId];
        delete previewTabs[tabId];
        delete dirtyByTab[tabId];
        delete saveErrorByTab[tabId];
        return { byTab, previewTabs, dirtyByTab, saveErrorByTab };
      }),

    ensure: async (tabId) => {
      const params = parseEditorTabId(tabId);
      if (!params || get().byTab[tabId]?.content !== undefined && get().byTab[tabId]?.content !== null) return;
      if (inflight.has(tabId)) return;
      inflight.add(tabId);
      patch(tabId, { loading: true, error: null });
      try {
        const content = await readWorkspaceFile(params.workspaceId, params.path);
        patch(tabId, { content, loading: false });
      } catch (e) {
        patch(tabId, { loading: false, error: String(e) });
      } finally {
        inflight.delete(tabId);
      }
    },

    markDirty: (tabId) =>
      set((s) =>
        s.dirtyByTab[tabId] ? s : { dirtyByTab: { ...s.dirtyByTab, [tabId]: true } },
      ),

    clearDirty: (tabId) =>
      set((s) => {
        const dirtyByTab = { ...s.dirtyByTab };
        delete dirtyByTab[tabId];
        return { dirtyByTab };
      }),

    save: async (tabId) => {
      const params = parseEditorTabId(tabId);
      const view = views.get(tabId);
      if (!params || !view) return;
      const doc = view.state.doc.toString();
      try {
        await writeWorkspaceFile(params.workspaceId, params.path, doc);
        // Disk now matches the editor — the post-save files:changed echo
        // must not clobber the live view (the component checks dirty).
        patch(tabId, { content: doc });
        set((s) => {
          const dirtyByTab = { ...s.dirtyByTab };
          delete dirtyByTab[tabId];
          const saveErrorByTab = { ...s.saveErrorByTab };
          delete saveErrorByTab[tabId];
          return { dirtyByTab, saveErrorByTab };
        });
      } catch (e) {
        set((s) => ({
          saveErrorByTab: { ...s.saveErrorByTab, [tabId]: String(e) },
        }));
      }
    },

    saveAll: async () => {
      await Promise.all(Object.keys(get().dirtyByTab).map((id) => get().save(id)));
    },
  };
});
