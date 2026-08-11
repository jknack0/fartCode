// Command registry wiring (E14-01): registers every keyboard command the
// app can actually run, with its default keymap entry and scope. Commands
// added by later epics register here too (one entry each).
//
// Terminal-first (design_handoff_v2 5a/5b): ⌘T resumes/reattaches the
// agent terminal, ⌘⇧T opens a shell in the task's worktree, ⌘J toggles the
// script drawer, ⌘. interrupts the live agent; ⌘⇧O opens OMP; ⌘⇧A opens
// the task's structured-chat conversation (E2-11-6).
import { CommandId, createRegistry, registerCommand } from "./registry";
import { useSidebar, visibleTaskOrder } from "../store/sidebar";
import {
  hostDependencyList,
  terminalListForTask,
  terminalOpen,
  terminalOpenAgent,
  terminalWrite,
} from "./tauri";
import {
  runAdvanceStep,
  runConfirmStep,
  runMoveToColumn,
  runOpenCardDetail,
} from "./taskPipeline";
import { useCommitState } from "../store/commit-state";
import { useConversations } from "../store/conversations";
import { useScripts, type ScriptType } from "../store/scripts";
import { useTabs, type PaneId } from "../store/tabs";
import { useUi } from "../store/ui";

export const registry = createRegistry();

/** Opens a new terminal tab (⌘⇧T / ⌘D — the only "new tab"). */
async function openTerminalTab(taskId: string, pane: PaneId): Promise<void> {
  const terminalId = await terminalOpen(taskId, 24, 80);
  useTabs.getState().addTab(taskId, pane, {
    id: terminalId,
    kind: "terminal",
    title: "Terminal",
  });
}

/** Opens (or focuses) the task's file-tree tab (E5-01). One per
 * workspace — the tab id encodes it, so addTab's dedupe-by-id makes a
 * second open a focus. */
export function openFileTree(taskId: string): void {
  const sidebar = useSidebar.getState();
  const task = sidebar.selectedProjectId
    ? (sidebar.tasksByProject[sidebar.selectedProjectId] ?? []).find((t) => t.id === taskId)
    : null;
  const project = sidebar.projects.find((p) => p.id === sidebar.selectedProjectId);
  const workspaceId = task?.workspaceId ?? project?.repositoryWorkspaceId ?? null;
  if (!workspaceId) return;
  const tabs = useTabs.getState();
  const pane: PaneId =
    tabs.panesByTask[taskId]?.right && tabs.activePaneByTask[taskId] === "right"
      ? "right"
      : "left";
  tabs.addTab(taskId, pane, {
    id: `files:${workspaceId}`,
    kind: "files",
    title: "Files",
  });
}

/** Opens the ⌘J drawer on a script's tab (7b — lifecycle scripts live in
 * the drawer, never in the tab bar), starting the script only when it has
 * never run. Reruns are explicit (`r` in the drawer / rerun label). */
export function openLifecycleScript(taskId: string, scriptType: ScriptType): void {
  const ui = useUi.getState();
  ui.setDrawerScript(scriptType);
  ui.setDrawerOpen(true);
  const run = useScripts.getState().byTask[taskId]?.[scriptType];
  if (!run?.terminalId) {
    void useScripts
      .getState()
      .rerun(taskId, scriptType)
      .catch((e) => console.error("script start failed", e));
  }
}

/** Resumes the task's agent terminal (⌘T, ADR-0033: one agent terminal per
 * task). Setup gates it (7b): while setup runs the resume refuses (the
 * empty state shows "Waiting on setup before starting…"), and after a
 * failed setup it opens the drawer on setup instead of spawning. A live
 * agent terminal reattaches under its own agent; otherwise the
 * default-agent app setting (7d `isDefault`) picks who to start, falling
 * back to claude. The tabs store dedupes by terminal id, so a second
 * resume focuses the existing tab. */
async function resumeAgentTab(taskId: string, pane: PaneId): Promise<void> {
  const setup = useScripts.getState().byTask[taskId]?.setup;
  if (setup?.running) return; // deferred — the backend launches on green
  if (setup && !setup.running && setup.exitCode !== null && setup.exitCode !== 0) {
    openLifecycleScript(taskId, "setup");
    return;
  }
  const terms = await terminalListForTask(taskId).catch(() => []);
  let agent = terms.find((t) => t.kind === "agent" && t.running)?.agent ?? null;
  if (!agent) {
    const deps = await hostDependencyList().catch(() => []);
    agent = deps.find((d) => d.isDefault)?.providerId ?? "claude";
  }
  const terminalId = await terminalOpenAgent(taskId, agent, 24, 80);
  useScripts.getState().noteAgentSpawn(taskId, terminalId);
  useTabs.getState().addTab(taskId, pane, {
    id: terminalId,
    kind: "terminal",
    title: agent,
  });
}

/** Opens an OMP agent terminal in the task's worktree (⌘⇧O). */
async function openOmpTab(taskId: string, pane: PaneId): Promise<void> {
  const terminalId = await terminalOpenAgent(taskId, "omp", 24, 80);
  useScripts.getState().noteAgentSpawn(taskId, terminalId);
  useTabs.getState().addTab(taskId, pane, {
    id: terminalId,
    kind: "terminal",
    title: "omp",
  });
}

/** Opens the task chat in the right sheet (⌘⇧A / header icon): chat mode
 * when the sheet is closed, switches changes mode → chat mode when open. */
export function openTaskChat(): void {
  const ui = useUi.getState();
  if (!ui.changesOpen) {
    ui.setTaskChatOpen(true);
    ui.setChangesOpen(true);
  } else if (ui.taskChatOpen) {
    ui.setChangesOpen(false); // chat mode → close the sheet
  } else {
    ui.setTaskChatOpen(true); // changes mode → chat mode
  }
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
    label: "Toggle project flyout",
    defaultKeys: ["⌘B", "⌘\\"],
    scope: "global",
    run: () => useUi.getState().toggleSidebarVisible(),
  });
  registerCommand(registry, {
    id: "toggle-right-panel",
    label: "Toggle resource monitor panel",
    // ⌘. is stop-agent (FLOWS.md §3.5 settled keymap).
    defaultKeys: ["⌘⇧."],
    scope: "global",
    run: () => {
      const ui = useUi.getState();
      ui.setResourceOpen(!ui.resourceOpen);
    },
  });
  // E4-03: the Changes sidebar (right-side panel). At project scope the
  // sheet alternates between changes and the PM chat (E17) — opening
  // changes leaves chat behind.
  registerCommand(registry, {
    id: "toggle-changes",
    label: "Toggle changes panel",
    defaultKeys: ["⌘⇧1"],
    scope: "global",
    run: () => {
      const ui = useUi.getState();
      const projectScope = !useSidebar.getState().selectedTaskId;
      if (!ui.changesOpen) {
        ui.setChangesOpen(true);
        if (projectScope) ui.setProjectChatOpen(false);
        else ui.setTaskChatOpen(false);
      } else if (projectScope && ui.projectChatOpen) {
        ui.setProjectChatOpen(false); // chat mode → changes mode
      } else if (!projectScope && ui.taskChatOpen) {
        ui.setTaskChatOpen(false); // chat mode → changes mode
      } else {
        ui.setChangesOpen(false);
      }
    },
  });

  // -- git plumbing (5d: palette-only — the Changes footer carries no
  // buttons, `fetch / pull / push in ⌘K` is the contract) -------------------
  const activeWorkspaceId = (): string | null => {
    const sb = useSidebar.getState();
    const task = sb.selectedTaskId
      ? Object.values(sb.tasksByProject)
          .flat()
          .find((t) => t.id === sb.selectedTaskId)
      : null;
    const project = sb.projects.find((p) => p.id === sb.selectedProjectId);
    return task?.workspaceId ?? project?.repositoryWorkspaceId ?? null;
  };
  const gitVerb = (
    id: Extract<CommandId, "git-fetch" | "git-pull" | "git-push" | "git-publish">,
    label: string,
    run: (workspaceId: string) => Promise<unknown>,
  ) => {
    registerCommand(registry, {
      id,
      label,
      defaultKeys: [],
      scope: "global",
      run: () => {
        const workspaceId = activeWorkspaceId();
        if (!workspaceId) return;
        void run(workspaceId).catch((e) => console.error(`${label} failed`, e));
      },
    });
  };
  gitVerb("git-fetch", "Git: fetch", (w) => useCommitState.getState().fetch(w));
  gitVerb("git-pull", "Git: pull", (w) => useCommitState.getState().pull(w));
  gitVerb("git-push", "Git: push", (w) => useCommitState.getState().push(w));
  gitVerb("git-publish", "Git: publish branch", (w) => useCommitState.getState().publish(w));

  // -- project view -----------------------------------------------------------
  registerCommand(registry, {
    id: "toggle-project-chat",
    label: "Toggle project chat panel",
    defaultKeys: ["⌘⇧2"],
    scope: "project-view",
    run: () => {
      const ui = useUi.getState();
      if (!ui.changesOpen) {
        ui.setProjectChatOpen(true);
        ui.setChangesOpen(true);
      } else if (ui.projectChatOpen) {
        ui.setChangesOpen(false); // chat mode → close the sheet
      } else {
        ui.setProjectChatOpen(true); // changes mode → chat mode
      }
    },
  });
  registerCommand(registry, {
    id: "add-task",
    label: "Add task",
    defaultKeys: ["⌘N"],
    // Global, not project-view: the flyout advertises ⌘N from the task view
    // too. run() already guards on a selected project.
    scope: "global",
    run: () => {
      const sb = useSidebar.getState();
      if (sb.selectedProjectId) useUi.getState().setCreateTaskTarget(sb.selectedProjectId);
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
    id: "resume-agent",
    label: "Resume the agent",
    defaultKeys: ["⌘T"],
    scope: "task-view",
    run: () => {
      const sb = useSidebar.getState();
      if (!sb.selectedTaskId) return;
      const taskId = sb.selectedTaskId;
      const pane = useTabs.getState().activePaneByTask[taskId] ?? "left";
      void resumeAgentTab(taskId, pane).catch((e) =>
        console.error("agent resume failed", e),
      );
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
    id: "toggle-drawer",
    label: "Toggle script drawer",
    defaultKeys: ["⌘J"],
    scope: "task-view",
    run: () => {
      const ui = useUi.getState();
      ui.setDrawerOpen(!ui.drawerOpen);
    },
  });
  registerCommand(registry, {
    id: "stop-agent",
    label: "Stop agent",
    defaultKeys: ["⌘."],
    scope: "task-view",
    // SIGINT to the live agent terminal — the terminal shows the agent's
    // own interrupt handling; the task view never fakes a stopped state.
    run: () => {
      const sb = useSidebar.getState();
      if (!sb.selectedTaskId) return;
      void terminalListForTask(sb.selectedTaskId)
        .then((terms) => {
          const live = terms.find((t) => t.kind === "agent" && t.running);
          if (live) return terminalWrite(live.id, "\x03");
        })
        .catch((e) => console.error("stop agent failed", e));
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
    run: () => openTaskChat(),
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
  // -- pipeline (task view, ADR-0037) ---------------------------------------
  //
  // The board's decision points, reachable from the surface the user is
  // actually on. Chords: ⌘⇧→ is the board's ⇧l "move the card one column
  // right" under the app's ⌘ prefix; ⌘⇧D is `dispatch`, the app's own verb
  // for firing a step (it opens the naming confirm, never spends on the
  // press, so the ⌘D adjacency cannot cost anything); ⌘⇧M is move; ⌘⇧I is
  // the issue. None collide with a registered chord or the reference
  // keymap (⌘T ⌘⇧T ⌘J ⌘. ⌘D ⌘W ⌘N ⌘K ⌘⌫ ⌘\ ⌘⇧1 ⌘⇧2 ⌘⇧. ⌘⌥↑/↓ ⌘1-9 ⌘↵
  // ⌘⇧A ⌘⇧O), and ⌘⇧3/4/5 are deliberately avoided — macOS owns them for
  // screenshots. Every one no-ops on a cardless (⌘N ad-hoc) task.
  registerCommand(registry, {
    id: "advance-step",
    label: "Advance to the next column",
    defaultKeys: ["⌘⇧→"],
    scope: "task-view",
    // ⌘⇧→ is select-to-end-of-line in a text field — the editor keeps it.
    skipInEditor: true,
    run: () => runAdvanceStep(),
  });
  registerCommand(registry, {
    id: "confirm-step",
    label: "Dispatch the parked step",
    defaultKeys: ["⌘⇧D"],
    scope: "task-view",
    skipInEditor: true,
    run: () => runConfirmStep(),
  });
  registerCommand(registry, {
    id: "move-to-column",
    label: "Move card to column…",
    defaultKeys: ["⌘⇧M"],
    scope: "task-view",
    skipInEditor: true,
    run: () => runMoveToColumn(),
  });
  registerCommand(registry, {
    id: "open-card-detail",
    label: "Open card detail",
    defaultKeys: ["⌘⇧I"],
    scope: "task-view",
    // Navigation, like its ⌘⇧1/⌘⇧2 siblings on the same right-panel slot —
    // it fires from a composer too.
    run: () => runOpenCardDetail(),
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
    defaultKeys: ["Ctrl+Shift+Tab"],
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
