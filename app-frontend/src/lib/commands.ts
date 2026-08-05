// Command registry wiring (E14-01): registers every keyboard command the
// app can actually run, with its default keymap entry and scope. Commands
// added by later epics register here too (one entry each).
//
// Terminal-first: ⌘T opens a shell in the task's worktree; ⌘⇧O opens OMP;
// ⌘⇧A opens the task's structured-chat conversation (E2-11-6).
import { CommandId, createRegistry, registerCommand } from "./registry";
import { ensureAcpConversation, focusConversationTab } from "./acp-conversation";
import { useSidebar, visibleTaskOrder } from "../store/sidebar";
import { terminalOpen, terminalOpenAgent } from "./tauri";
import { useConversations } from "../store/conversations";
import { useTabs, type PaneId } from "../store/tabs";
import { useUi } from "../store/ui";

export const registry = createRegistry();

/** Opens a new terminal tab (⌘T / ⌘⇧T — the only "new tab"). */
async function openTerminalTab(taskId: string, pane: PaneId): Promise<void> {
  const terminalId = await terminalOpen(taskId, 24, 80);
  useTabs.getState().addTab(taskId, pane, {
    id: terminalId,
    kind: "terminal",
    title: "Terminal",
  });
}

/** Opens an OMP agent terminal in the task's worktree (⌘⇧O). */
async function openOmpTab(taskId: string, pane: PaneId): Promise<void> {
  const terminalId = await terminalOpenAgent(taskId, "omp", 24, 80);
  useTabs.getState().addTab(taskId, pane, {
    id: terminalId,
    kind: "terminal",
    title: "omp",
  });
}

/** Opens (or focuses) the task's ACP conversation tab (⌘⇧A). */
async function openConversationTab(
  projectId: string,
  taskId: string,
  pane: PaneId,
): Promise<void> {
  const conv = await ensureAcpConversation(projectId, taskId);
  if (conv) focusConversationTab(taskId, conv.id, pane);
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
  // E4-03: the Changes sidebar (right-side panel in the task view).
  registerCommand(registry, {
    id: "toggle-changes",
    label: "Toggle changes panel",
    defaultKeys: ["⌘⇧1"],
    scope: "global",
    run: () => {
      const ui = useUi.getState();
      ui.setChangesOpen(!ui.changesOpen);
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
    id: "new-terminal",
    label: "New terminal",
    defaultKeys: ["⌘T", "⌘⇧T"],
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
    id: "new-terminal-right-split",
    label: "New terminal in right split",
    defaultKeys: ["⌘D"],
    scope: "task-view",
    run: () => {
      const sb = useSidebar.getState();
      const tabs = useTabs.getState();
      if (!sb.selectedTaskId) return;
      if (!tabs.panesByTask[sb.selectedTaskId]?.right) {
        // toggleSplit already spawns a fresh shell in the new right pane —
        // one PTY drives one xterm surface, so nothing else to add.
        void tabs.toggleSplit(sb.selectedTaskId);
      } else {
        void openTerminalTab(sb.selectedTaskId, "right").catch((e) =>
          console.error("terminal open failed", e),
        );
      }
    },
  });
  registerCommand(registry, {
    id: "open-omp",
    label: "Open OMP terminal",
    defaultKeys: ["⌘⇧O"],
    scope: "task-view",
    run: () => {
      const sb = useSidebar.getState();
      if (!sb.selectedTaskId) return;
      const taskId = sb.selectedTaskId;
      const pane = useTabs.getState().activePaneByTask[taskId] ?? "left";
      void openOmpTab(taskId, pane).catch((e) =>
        console.error("omp open failed", e),
      );
    },
  });
  registerCommand(registry, {
    id: "open-conversation",
    label: "Open agent conversation",
    defaultKeys: ["⌘⇧A"],
    scope: "task-view",
    run: () => {
      const sb = useSidebar.getState();
      if (!sb.selectedTaskId || !sb.selectedProjectId) return;
      const taskId = sb.selectedTaskId;
      const pane = useTabs.getState().activePaneByTask[taskId] ?? "left";
      void openConversationTab(sb.selectedProjectId, taskId, pane).catch((e) =>
        console.error("conversation open failed", e),
      );
    },
  });
  registerCommand(registry, {
    id: "send-context",
    label: "Send prompt to agent",
    defaultKeys: ["⌘Enter"],
    scope: "task-view",
    // Fires even while an editor/input is focused — ⌘Enter from the
    // composer submits. TUI conversations keep the terminal path
    // byte-identical: the command routes ONLY when the task's conversation
    // resolved to the ACP runtime (E2-11-5 provider decision).
    run: () => {
      const sb = useSidebar.getState();
      if (!sb.selectedTaskId) return;
      const conversations = useConversations.getState();
      const acp = conversations.activeAcp(sb.selectedTaskId);
      if (!acp) return; // TUI path — untouched by design.
      const text = (conversations.drafts[acp.id] ?? "").trim();
      if (!text) return;
      void conversations.sendPrompt(acp.id, text).catch((e) =>
        console.error("acp send failed", e),
      );
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
      // Spawns a fresh shell in the new right pane (or collapses it).
      void useTabs.getState().toggleSplit(sb.selectedTaskId);
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

  // -- modal --------------------------------------------------------------------
  registerCommand(registry, {
    id: "close-modal",
    label: "Close modal",
    defaultKeys: ["Escape"],
    scope: "modal",
    run: () => useUi.getState().closeTopModal(),
  });
}
