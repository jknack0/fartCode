// Diff tab opening (E4-04): the seam between the Changes sidebar (E4-03)
// and the diff tab kind. The tab id encodes the diff params
// (`diff:<side>:<workspaceId>:<path>`) so tabs restored from view-state
// after a restart re-parse them without any persisted sidecar.
import { useDiffs, type DiffParams } from "../store/diffs";
import { useTabs, type PaneId } from "../store/tabs";
import type { DiffSide } from "../lib/tauri";

export interface OpenDiffTabArgs {
  taskId: string;
  workspaceId: string;
  path: string;
  origPath?: string | null;
  side: DiffSide;
  /** true = preview tab (replaced by the next preview open); false = persistent. */
  preview?: boolean;
}

export function diffTabId(workspaceId: string, side: DiffSide, path: string): string {
  return `diff:${side}:${workspaceId}:${path}`;
}

/** Inverse of `diffTabId`. `workspaceId` is a UUID (no colons); the path
 * is everything after the third colon, so colon-containing paths survive. */
export function parseDiffTabId(
  id: string,
): { workspaceId: string; side: DiffSide; path: string } | null {
  const parts = id.match(/^diff:(staged|unstaged):([^:]+):(.+)$/);
  if (!parts) return null;
  return { side: parts[1] as DiffSide, workspaceId: parts[2], path: parts[3] };
}

export function diffTabTitle(path: string, side: DiffSide): string {
  const base = path.split("/").pop() ?? path;
  return side === "staged" ? `${base} (staged)` : base;
}

/** Opens (or focuses) a diff tab for one file. Preview opens replace the
 * pane's current preview diff tab; a persistent open of the preview's file
 * flips it to persistent instead of duplicating. */
export function openDiffTab(args: OpenDiffTabArgs): void {
  const { taskId, workspaceId, path, side } = args;
  const origPath = args.origPath ?? null;
  const preview = args.preview ?? true;
  const id = diffTabId(workspaceId, side, path);
  const params: DiffParams = { workspaceId, path, origPath, side };

  const tabs = useTabs.getState();
  const panes = tabs.panesByTask[taskId];
  if (!panes) return; // task view not mounted — nothing to open into
  const pane: PaneId =
    panes.right && tabs.activePaneByTask[taskId] === "right" ? "right" : "left";

  const diffs = useDiffs.getState();
  const existing = panes[pane]?.tabs.find((t) => t.id === id);
  if (existing) {
    if (!preview) diffs.setPreview(id, false);
    tabs.setActiveTab(taskId, pane, id);
    return;
  }

  if (preview) {
    const currentPreview = panes[pane]?.tabs.find(
      (t) => t.kind === "diff" && diffs.previewTabs[t.id],
    );
    if (currentPreview) {
      // The tab id encodes the file, so retargeting = close + open fresh.
      tabs.closeTab(taskId, pane, currentPreview.id);
      diffs.dropTab(currentPreview.id);
    }
  }

  diffs.setParams(id, params);
  diffs.setPreview(id, preview);
  tabs.addTab(taskId, pane, {
    id,
    kind: "diff",
    title: diffTabTitle(path, side),
  });
}

/** Test seam (browser smoke), mirrors `window.__conversationsStore`. */
declare global {
  interface Window {
    __diffTabs?: { open: typeof openDiffTab };
  }
}
if (typeof window !== "undefined") window.__diffTabs = { open: openDiffTab };
