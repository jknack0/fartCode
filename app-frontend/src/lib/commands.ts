// Command registry wiring (E14-01): registers every keyboard command the
// app can actually run, with its default keymap entry and scope. Commands
// added by later epics register here too (one entry each).
import { CommandId, createRegistry, registerCommand } from "./registry";
import { useConversations } from "../store/conversations";
import { useSidebar, visibleTaskOrder } from "../store/sidebar";
import { terminalOpen } from "./tauri";
import { useTabs, type PaneId } from "../store/tabs";
import { useUi } from "../store/ui";

export const registry = createRegistry();

/** Create a conversation and open it as a tab in the given pane (E2-10). */
/** Opens a new terminal tab (⌘⇧T and the task-open default). */
async function openTerminalTab(taskId: string, pane: PaneId): Promise<void> {
  const terminalId = await terminalOpen(taskId, 24, 80);
  useTabs.getState().addTab(taskId, pane, {
    id: terminalId,
    kind: "terminal",
    title: "Terminal",
  });
}

async function openConversationTab(
  taskId: string,
  projectId: string,
  pane: PaneId,
): Promise<void> {
  const conv = await useConversations
    .getState()
    .create(taskId, projectId, undefined, "New conversation", undefined);
  useTabs.getState().addTab(taskId, pane, {
    id: conv.id,
    kind: "conversation",
    title: conv.title,
  });
}

function switchTask(dir: 1 | -1): void {
  const sb = useSidebar.getState();
  const order = visibleTaskOrder(sb);
  if (order.length === 0) return;
  const idx = order.findIndex((t) => t.id === sb.selectedTaskId);
  const next =
    idx === -1
      ? order[dir === 1 ? 0 : order.length - 1]
      : order[(idx + dir + order.length) % order.length];
  if (next.id !== sb.selectedTaskId) sb.switchToTask(next);
}

export function registerAllCommands(): void {
  // -- global ---------------------------------------------------------------
  registerCommand(registry, {
    id: "open-command-palette",
    label: "Command palette",
    defaultKeys: ["⌘K"],
    scope: "global",
    run: () => {
      const ui = useUi.getState();
      ui.setPaletteOpen(!ui.paletteOpen);
    },
  });
  registerCommand(registry, {
    id: "open-settings",
    label: "Open settings",
    defaultKeys: ["⌘,"],
    scope: "global",
    run: () => useUi.getState().setSettingsOpen(true),
  });
  registerCommand(registry, {
    id: "new-project",
    label: "Add project",
    defaultKeys: ["⌘⇧N"],
    scope: "global",
    run: () => useUi.getState().setCreateProjectOpen(true),
  });
  registerCommand(registry, {
    id: "toggle-sidebar",
    label: "Toggle sidebar",
    defaultKeys: ["⌘B"],
    scope: "global",
    run: () => useUi.getState().toggleSidebarVisible(),
  });
  registerCommand(registry, {
    id: "toggle-right-panel",
    label: "Toggle resource monitor panel",
    defaultKeys: ["⌘."],
    scope: "global",
    run: () => {
      const ui = useUi.getState();
      ui.setResourceOpen(!ui.resourceOpen);
    },
  });

  // -- project view -----------------------------------------------------------
  registerCommand(registry, {
    id: "add-task",
    label: "Add task",
    defaultKeys: ["⌘N"],
    scope: "project-view",
    run: () => {
      const sb = useSidebar.getState();
      if (sb.selectedProjectId) void sb.createTask(sb.selectedProjectId);
    },
  });

  // -- task view --------------------------------------------------------------
  registerCommand(registry, {
    id: "delete-task",
    label: "Delete task",
    defaultKeys: ["⌘Backspace"],
    scope: "task-view",
    skipInEditor: true,
    run: () => {
      const sb = useSidebar.getState();
      if (!sb.selectedTaskId || !sb.selectedProjectId) return;
      useUi.getState().setDeleteTaskTarget({
        projectId: sb.selectedProjectId,
        taskId: sb.selectedTaskId,
      });
    },
  });
  registerCommand(registry, {
    id: "new-conversation",
    label: "New conversation",
    defaultKeys: ["⌘T"],
    scope: "task-view",
    run: () => {
      const sb = useSidebar.getState();
      if (!sb.selectedTaskId || !sb.selectedProjectId) return;
      const pane = useTabs.getState().activePaneByTask[sb.selectedTaskId] ?? "left";
      void openConversationTab(sb.selectedTaskId, sb.selectedProjectId, pane);
    },
  });
  registerCommand(registry, {
    id: "new-terminal",
    label: "New terminal",
    defaultKeys: ["⌘⇧T"],
    scope: "task-view",
    run: () => {
      const sb = useSidebar.getState();
      if (!sb.selectedTaskId) return;
      const taskId = sb.selectedTaskId;
      const pane = useTabs.getState().activePaneByTask[taskId] ?? "left";
      void openTerminalTab(taskId, pane).catch((e) =>
        console.error("terminal open failed", e),
      );
    },
  });
  registerCommand(registry, {
    id: "new-conversation-right-split",
    label: "New conversation in right split",
    defaultKeys: ["⌘D"],
    scope: "task-view",
    run: () => {
      const sb = useSidebar.getState();
      const tabs = useTabs.getState();
      if (!sb.selectedTaskId || !sb.selectedProjectId) return;
      if (!tabs.panesByTask[sb.selectedTaskId]?.right) {
        tabs.toggleSplit(sb.selectedTaskId, taskName(sb.selectedTaskId));
      }
      void openConversationTab(sb.selectedTaskId, sb.selectedProjectId, "right");
    },
  });
  registerCommand(registry, {
    id: "previous-task",
    label: "Previous task",
    defaultKeys: ["⌘⌥↑"],
    scope: "task-view",
    skipInEditor: true,
    run: () => switchTask(-1),
  });
  registerCommand(registry, {
    id: "next-task",
    label: "Next task",
    defaultKeys: ["⌘⌥↓"],
    scope: "task-view",
    skipInEditor: true,
    run: () => switchTask(1),
  });
  registerCommand(registry, {
    id: "close-tab",
    label: "Close tab",
    defaultKeys: ["⌘W"],
    scope: "task-view",
    skipInEditor: true,
    run: () => {
      const sb = useSidebar.getState();
      const tabs = useTabs.getState();
      if (!sb.selectedTaskId) return;
      const pane = tabs.activePaneByTask[sb.selectedTaskId] ?? "left";
      const activeId = tabs.panesByTask[sb.selectedTaskId]?.[pane]?.activeId;
      if (activeId) tabs.closeTab(sb.selectedTaskId, pane, activeId);
    },
  });
  registerCommand(registry, {
    id: "split-pane",
    label: "Split pane",
    defaultKeys: ["⌘\\"],
    scope: "task-view",
    skipInEditor: true,
    run: () => {
      const sb = useSidebar.getState();
      if (!sb.selectedTaskId) return;
      useTabs.getState().toggleSplit(sb.selectedTaskId, taskName(sb.selectedTaskId));
    },
  });
  registerCommand(registry, {
    id: "next-tab",
    label: "Next tab (wrap)",
    defaultKeys: ["Ctrl+Tab"],
    scope: "task-view",
    run: () => {
      const sb = useSidebar.getState();
      const tabs = useTabs.getState();
      if (!sb.selectedTaskId) return;
      const pane = tabs.activePaneByTask[sb.selectedTaskId] ?? "left";
      tabs.cycleTab(sb.selectedTaskId, pane, 1);
    },
  });
  registerCommand(registry, {
    id: "previous-tab",
    label: "Previous tab (wrap)",
    defaultKeys: ["Ctrl+⇧+Tab"],
    scope: "task-view",
    run: () => {
      const sb = useSidebar.getState();
      const tabs = useTabs.getState();
      if (!sb.selectedTaskId) return;
      const pane = tabs.activePaneByTask[sb.selectedTaskId] ?? "left";
      tabs.cycleTab(sb.selectedTaskId, pane, -1);
    },
  });
  for (let n = 1; n <= 9; n++) {
    registerCommand(registry, {
      id: `jump-to-tab-${n}` as CommandId,
      label: `Jump to tab ${n}`,
      defaultKeys: [`⌘${n}`],
      scope: "task-view",
      skipInEditor: true,
      run: () => {
        const sb = useSidebar.getState();
        const tabs = useTabs.getState();
        if (!sb.selectedTaskId) return;
        const pane = tabs.activePaneByTask[sb.selectedTaskId] ?? "left";
        tabs.jumpToTab(sb.selectedTaskId, pane, n);
      },
    });
  }

  // -- conversation view -------------------------------------------------------
  registerCommand(registry, {
    id: "add-and-send-context",
    label: "Add and send context",
    defaultKeys: ["⌘Enter"],
    scope: "conversation-view",
    run: () => useConversations.getState().send(),
  });
  registerCommand(registry, {
    id: "add-context",
    label: "Add context menu",
    defaultKeys: ["⌘⇧A"],
    scope: "conversation-view",
    run: () => {
      const raw = prompt("Add context - path to a file, or '@ text':");
      if (!raw) return;
      const convs = useConversations.getState();
      if (raw.startsWith("@")) convs.addContextPrompt(raw.slice(1).trim());
      else convs.addContextFile(raw.trim());
    },
  });

  // -- modal --------------------------------------------------------------------
  registerCommand(registry, {
    id: "close-modal",
    label: "Close modal / exit command palette",
    defaultKeys: ["Escape"],
    scope: "modal",
    run: () => useUi.getState().closeTopModal(),
  });
}

function taskName(taskId: string): string {
  const s = useSidebar.getState();
  for (const p of s.projects) {
    const t = (s.tasksByProject[p.id] ?? []).find((t) => t.id === taskId);
    if (t) return t.name;
  }
  return "Task";
}
