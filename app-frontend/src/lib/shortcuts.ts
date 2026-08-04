// Scoped task-view shortcuts (E2-10). E14-01 later replaces this with the
// full command registry + user keymap; the scoping rules enforced here are
// the ones E2-10 owns: task-view scope, editor exemption, and Ctrl+Tab
// wrap-around that stays active inside terminal/editor/browser.
import { useEffect } from "react";
import { useConversations } from "../store/conversations";
import { useSidebar, visibleTaskOrder } from "../store/sidebar";
import { useTabs, type PaneId } from "../store/tabs";

/** Create a conversation and open it as a tab in the given pane. */
async function openConversationTab(
  taskId: string,
  projectId: string,
  pane: PaneId,
) {
  const conv = await useConversations
    .getState()
    .create(taskId, projectId, undefined, "New conversation", undefined);
  useTabs.getState().addTab(taskId, pane, {
    id: conv.id,
    kind: "conversation",
    title: conv.title,
  });
}

function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  const tag = target.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || target.isContentEditable;
}

/** Modal overlays consume their own keys (scope precedence: modal first). */
function inModalOverlay(target: EventTarget | null): boolean {
  return (
    target instanceof HTMLElement &&
    !!target.closest(".modal-backdrop, .modal, .palette, .resource-panel")
  );
}

function taskNameFor(taskId: string): string {
  const s = useSidebar.getState();
  for (const p of s.projects) {
    const t = (s.tasksByProject[p.id] ?? []).find((t) => t.id === taskId);
    if (t) return t.name;
  }
  return "Task";
}

export function useTaskShortcuts(): void {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const { selectedTaskId, selectedProjectId } = useSidebar.getState();
      const tabs = useTabs.getState();

      const editable = isEditableTarget(e.target);
      const modal = inModalOverlay(e.target);

      // Ctrl+Tab / Ctrl+Shift+Tab: cycle the active pane's tabs with
      // wrap-around. Active even inside terminal/editor/browser inputs
      // (browser semantics) — the only shortcut that bypasses the
      // editor exemption.
      if (e.ctrlKey && !e.metaKey && !e.altKey && e.key === "Tab") {
        e.preventDefault();
        if (!selectedTaskId) return;
        const pane = tabs.activePaneByTask[selectedTaskId] ?? "left";
        tabs.cycleTab(selectedTaskId, pane, e.shiftKey ? -1 : 1);
        return;
      }

      // Everything below is task-view scope.
      if (modal || !selectedTaskId) return;

      // ⌘⌥↑/↓: previous/next task in visible tree order (pinned first,
      // collapsed projects skipped), wrap-around. Editor-exempt.
      if (
        e.metaKey &&
        !e.ctrlKey &&
        e.altKey &&
        !e.shiftKey &&
        (e.key === "ArrowUp" || e.key === "ArrowDown")
      ) {
        if (editable) return;
        e.preventDefault();
        const order = visibleTaskOrder(useSidebar.getState());
        if (order.length === 0) return;
        const idx = order.findIndex((t) => t.id === selectedTaskId);
        const dir = e.key === "ArrowDown" ? 1 : -1;
        const next =
          idx === -1
            ? order[dir === 1 ? 0 : order.length - 1]
            : order[(idx + dir + order.length) % order.length];
        if (next.id !== selectedTaskId) {
          useSidebar.getState().switchToTask(next);
        }
        return;
      }

      const pane = tabs.activePaneByTask[selectedTaskId] ?? "left";
      const paneState = tabs.panesByTask[selectedTaskId]?.[pane];

      // ⌘T new conversation tab · ⌘D new conversation in a right split.
      if (e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey) {
        const key = e.key.toLowerCase();
        if (key === "t") {
          e.preventDefault();
          if (selectedProjectId) {
            void openConversationTab(selectedTaskId, selectedProjectId, pane);
          }
          return;
        }
        if (key === "d") {
          e.preventDefault();
          const tabsNow = useTabs.getState();
          if (!tabsNow.panesByTask[selectedTaskId]?.right) {
            tabsNow.toggleSplit(selectedTaskId, taskNameFor(selectedTaskId));
          }
          if (selectedProjectId) {
            void openConversationTab(selectedTaskId, selectedProjectId, "right");
          }
          return;
        }
      }

      // ⌘1–9 jump to tab · ⌘W close · ⌘\ split. Editor-exempt (the editor
      // consumes these while focused).
      if (editable || !paneState) return;
      if (e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey) {
        const key = e.key.toLowerCase();
        if (key >= "1" && key <= "9") {
          e.preventDefault();
          tabs.jumpToTab(selectedTaskId, pane, Number(key));
          return;
        }
        if (key === "w") {
          if (paneState.activeId) {
            e.preventDefault();
            tabs.closeTab(selectedTaskId, pane, paneState.activeId);
          }
          return;
        }
        if (key === "\\") {
          e.preventDefault();
          tabs.toggleSplit(selectedTaskId, taskNameFor(selectedTaskId));
          return;
        }
      }
    };

    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}
