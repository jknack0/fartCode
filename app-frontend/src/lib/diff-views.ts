// Live CodeMirror handles for open diff tabs (E4-05). Non-serializable, so
// they live in a module map rather than the zustand store: the save path
// (⌘S) reads the editor's current document, and the refresh logic compares
// against it to avoid clobbering an in-progress edit.
import type { EditorView } from "@codemirror/view";
import { MergeView } from "@codemirror/merge";

const views = new Map<string, EditorView | MergeView>();

export function registerDiffView(tabId: string, view: EditorView | MergeView): void {
  views.set(tabId, view);
}

export function unregisterDiffView(tabId: string, view: EditorView | MergeView): void {
  if (views.get(tabId) === view) views.delete(tabId);
}

export function getDiffView(tabId: string): EditorView | MergeView | undefined {
  return views.get(tabId);
}

/** The two documents a diff tab shows: `a` = baseline (HEAD/index), `b` =
 * worktree. Single-document views (added/deleted) report only `b`. */
export function diffViewDocs(view: EditorView | MergeView): { a: string | null; b: string } {
  if (view instanceof MergeView) {
    return { a: view.a.state.doc.toString(), b: view.b.state.doc.toString() };
  }
  return { a: null, b: view.state.doc.toString() };
}

/** The editable side's document (worktree side). */
export function diffViewWorktreeDoc(view: EditorView | MergeView): string {
  return diffViewDocs(view).b;
}

/** Test seam (browser smoke), mirrors `window.__conversationsStore`. */
declare global {
  interface Window {
    __diffViews?: Map<string, EditorView | MergeView>;
  }
}
if (typeof window !== "undefined") window.__diffViews = views;
