// Editor tab opening (E5-02): the seam between the file tree (E5-01) and
// the file-editor tab kind. Tab id encodes the params
// (`edit:<workspaceId>:<path>`) — restored tabs re-parse, no sidecar
// (diff-tabs' exact pattern). Preview semantics too: a preview open
// replaces the pane's current preview editor; edit or re-open flips
// persistent.
import { onOpenFile } from "./open-file";
import { useEditors } from "../store/editors";
import { useTabs, type PaneId } from "../store/tabs";

export function editorTabId(workspaceId: string, path: string): string {
  return `edit:${workspaceId}:${path}`;
}

/** Inverse of `editorTabId`. workspaceId is a UUID (no colons); the path
 * is everything after the second colon. */
export function parseEditorTabId(
  id: string,
): { workspaceId: string; path: string } | null {
  const m = id.match(/^edit:([^:]+):(.+)$/);
  return m ? { workspaceId: m[1], path: m[2] } : null;
}

export interface OpenEditorTabArgs {
  taskId: string;
  workspaceId: string;
  path: string;
  /** true = preview tab (replaced by the next preview open). */
  preview?: boolean;
}

export function openEditorTab(args: OpenEditorTabArgs): void {
  const { taskId, workspaceId, path } = args;
  const preview = args.preview ?? true;
  const id = editorTabId(workspaceId, path);

  const tabs = useTabs.getState();
  const panes = tabs.panesByTask[taskId];
  if (!panes) return;
  const pane: PaneId =
    panes.right && tabs.activePaneByTask[taskId] === "right" ? "right" : "left";

  const editors = useEditors.getState();
  const existing = (panes.left.tabs.some((t) => t.id === id) && "left") ||
    (panes.right?.tabs.some((t) => t.id === id) && "right") || null;
  if (existing) {
    // A re-open of the previewed file flips it persistent.
    if (!preview || editors.previewTabs[id]) editors.setPreview(id, false);
    tabs.setActiveTab(taskId, existing, id);
    return;
  }

  if (preview) {
    const currentPreview = panes[pane]?.tabs.find(
      (t) => t.kind === "file-editor" && editors.previewTabs[t.id],
    );
    if (currentPreview) {
      tabs.closeTab(taskId, pane, currentPreview.id);
      editors.dropTab(currentPreview.id);
    }
  }

  editors.setPreview(id, preview);
  tabs.addTab(taskId, pane, {
    id,
    kind: "file-editor",
    title: path.split("/").pop() ?? path,
  });
}

/** Wires tree clicks (E5-01 open-file intent) to editor tabs. Call once at
 * app boot; returns the unsubscribe. */
export function wireOpenFileIntents(): () => void {
  return onOpenFile(({ taskId, workspaceId, path }) =>
    openEditorTab({ taskId, workspaceId, path, preview: true }),
  );
}
