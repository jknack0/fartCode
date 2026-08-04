// Per-task tab state (E2-10): each task has a left pane (always) and an
// optional right pane (⌘\ split). Every pane owns a tab list + active tab.
// Tab state persists under `view-state:task:<id>:tabs` — the exact key the
// backend tears down with the task (E2-09) and prunes on boot (E1-08).
//
// Today a tab is a conversation (the one registered kind). Creating a
// conversation adds a tab; closing one deletes the conversation (see the
// registry's `onClose`). Future kinds (diff, browser, file-editor) register
// in `lib/tab-registry.tsx` and flow through the same machinery.
import { create } from "zustand";
import {
  getViewState,
  listConversations,
  onAdeEvent,
  setViewState,
  terminalOpen,
} from "../lib/tauri";
import { isTabKind, TAB_KINDS, type Tab } from "../lib/tab-registry";
import { useConversations } from "./conversations";

export type PaneId = "left" | "right";

export interface Pane {
  tabs: Tab[];
  activeId: string | null;
}

interface TaskPanes {
  left: Pane;
  right: Pane | null;
}

interface SavedTabs {
  left?: Pane;
  right?: Pane | null;
  activePane?: PaneId;
}

interface TabsState {
  panesByTask: Record<string, TaskPanes>;
  activePaneByTask: Record<string, PaneId>;
  loadedByTask: Record<string, boolean>;

  ensureTabs: (taskId: string, taskName: string) => Promise<void>;
  addTab: (taskId: string, pane: PaneId, tab: Tab) => void;
  closeTab: (taskId: string, pane: PaneId, tabId: string) => void;
  jumpToTab: (taskId: string, pane: PaneId, n: number) => void;
  cycleTab: (taskId: string, pane: PaneId, dir: 1 | -1) => void;
  toggleSplit: (taskId: string, taskName: string) => void;
  setActivePane: (taskId: string, pane: PaneId) => void;
  setActiveTab: (taskId: string, pane: PaneId, tabId: string) => void;
  dropTask: (taskId: string) => void;
}

/// Validates a persisted pane: registry-known kinds only, at least one tab,
/// active id present.
function sanitizePane(pane: Pane | undefined): Pane | null {
  if (!pane) return null;
  const tabs = pane.tabs.filter(
    (t): t is Tab =>
      typeof t?.id === "string" &&
      typeof t?.title === "string" &&
      typeof t?.kind === "string" &&
      isTabKind(t.kind) &&
      // Ephemeral kinds (terminals) do not survive a reload — their PTY is
      // gone; drop them from restored state.
      !TAB_KINDS[t.kind].ephemeral,
  );
  if (tabs.length === 0) return null;
  const activeId = pane.activeId && tabs.some((t) => t.id === pane.activeId)
    ? pane.activeId
    : tabs[0].id;
  return { tabs, activeId };
}

function persist(taskId: string, panes: TaskPanes, activePane: PaneId) {
  void setViewState(`view-state:task:${taskId}:tabs`, {
    left: panes.left,
    right: panes.right,
    activePane,
  });
}

const hydrating = new Set<string>();

export const useTabs = create<TabsState>((set, get) => ({
  panesByTask: {},
  activePaneByTask: {},
  loadedByTask: {},

  ensureTabs: async (taskId, taskName) => {
    if (get().loadedByTask[taskId] || hydrating.has(taskId)) return;
    hydrating.add(taskId);
    try {
      const [saved, convs] = await Promise.all([
        getViewState(`view-state:task:${taskId}:tabs`) as Promise<SavedTabs | null>,
        listConversations(taskId),
      ]);
      const byId = new Map(convs.map((c) => [c.id, c]));

      // Reconcile persisted tabs: keep only registered kinds with live
      // backing. Conversation tabs are never force-added — they exist only
      // when the user opened them (terminal-first default below).
      const reconcile = (pane: Pane | undefined): Pane => {
        const clean = sanitizePane(pane);
        const kept = clean
          ? clean.tabs.filter((t) => t.kind !== "conversation" || byId.has(t.id))
          : [];
        if (kept.length === 0) return { tabs: [], activeId: null };
        const activeId =
          clean && clean.tabs.some((t) => t.id === clean.activeId)
            ? clean.activeId
            : kept[0].id;
        return { tabs: kept, activeId };
      };

      let left = reconcile(saved?.left);
      let right = saved?.right ? reconcile(saved.right) : null;
      if (right && right.tabs.length === 0) right = null;
      if (left.tabs.length === 0) {
        // Terminal-first: a task's default surface is a shell in its
        // worktree; conversation tabs are summoned explicitly (⌘T), never
        // auto-opened. (E2-12: work inside ade.)
        try {
          const terminalId = await terminalOpen(taskId, 24, 80);
          left = {
            tabs: [{ id: terminalId, kind: "terminal", title: taskName }],
            activeId: terminalId,
          };
        } catch (e) {
          console.warn("terminal_open failed on task open:", e);
          left = { tabs: [], activeId: null };
        }
      }
      const panes: TaskPanes = { left, right };
      set((s) => ({
        panesByTask: { ...s.panesByTask, [taskId]: panes },
        activePaneByTask: {
          ...s.activePaneByTask,
          [taskId]: saved?.activePane ?? "left",
        },
        loadedByTask: { ...s.loadedByTask, [taskId]: true },
      }));
    } finally {
      hydrating.delete(taskId);
    }
  },

  addTab: (taskId, pane, tab) => {
    set((s) => {
      const old = s.panesByTask[taskId];
      if (!old || !old[pane]) return s;
      if (old[pane].tabs.some((t) => t.id === tab.id)) return s;
      const target: Pane = { tabs: [...old[pane].tabs, tab], activeId: tab.id };
      const panes: TaskPanes = {
        left: pane === "left" ? target : old.left,
        right: pane === "right" ? target : old.right ? { ...old.right } : null,
      };
      persist(taskId, panes, pane);
      return {
        panesByTask: { ...s.panesByTask, [taskId]: panes },
        activePaneByTask: { ...s.activePaneByTask, [taskId]: pane },
      };
    });
  },

  closeTab: (taskId, pane, tabId) => {
    const closedTab = get().panesByTask[taskId]?.[pane]?.tabs.find(
      (t) => t.id === tabId,
    );
    let newActiveId: string | null = null;

    set((s) => {
      const old = s.panesByTask[taskId];
      if (!old || !old[pane]) return s;
      const closed = old[pane].tabs.findIndex((t) => t.id === tabId);
      if (closed === -1) return s;

      const remaining = old[pane].tabs.filter((t) => t.id !== tabId);
      let activePane = s.activePaneByTask[taskId] ?? "left";

      // Closing the last tab of the split pane collapses the split.
      if (pane === "right" && remaining.length === 0) {
        const panes: TaskPanes = { left: old.left, right: null };
        activePane = "left";
        newActiveId = old.left.activeId ?? old.left.tabs[0]?.id ?? null;
        persist(taskId, panes, activePane);
        return {
          panesByTask: { ...s.panesByTask, [taskId]: panes },
          activePaneByTask: { ...s.activePaneByTask, [taskId]: activePane },
        };
      }

      const activeId =
        old[pane].activeId === tabId
          ? remaining.length > 0
            ? remaining[Math.min(closed, remaining.length - 1)].id
            : null
          : old[pane].activeId;
      newActiveId = activeId;
      const target: Pane = { tabs: remaining, activeId };
      const panes: TaskPanes = {
        left: pane === "left" ? target : old.left,
        right: pane === "right" ? target : old.right ? { ...old.right } : null,
      };
      persist(taskId, panes, activePane);
      return { panesByTask: { ...s.panesByTask, [taskId]: panes } };
    });

    // The tab is the conversation's only surface, so closing its LAST
    // reference deletes the conversation (⌘W); split duplicates keep it.
    if (closedTab?.kind !== "conversation") return;
    const after = get().panesByTask[taskId];
    const stillReferenced = [after?.left.tabs ?? [], after?.right?.tabs ?? []]
      .flat()
      .some((t) => t.id === tabId);
    if (stillReferenced) return;
    const convs = useConversations.getState();
    const conv = convs.conversations.find((c) => c.id === tabId);
    if (!conv) return;
    void convs.remove(conv.projectId, conv.taskId ?? taskId, conv.id);
    if (convs.activeId === tabId && newActiveId) {
      convs.setActive(newActiveId);
    }
  },

  jumpToTab: (taskId, pane, n) => {
    set((s) => {
      const old = s.panesByTask[taskId];
      const target = old?.[pane];
      if (!old || !target || target.tabs.length === 0) return s;
      const tab = target.tabs[n - 1];
      if (!tab) return s;
      const updated: Pane = { ...target, activeId: tab.id };
      const panes: TaskPanes = {
        left: pane === "left" ? updated : old.left,
        right: pane === "right" ? updated : old.right ? { ...old.right } : null,
      };
      persist(taskId, panes, s.activePaneByTask[taskId] ?? "left");
      return { panesByTask: { ...s.panesByTask, [taskId]: panes } };
    });
  },

  cycleTab: (taskId, pane, dir) => {
    set((s) => {
      const old = s.panesByTask[taskId];
      const target = old?.[pane];
      if (!old || !target || target.tabs.length === 0) return s;
      const idx = target.tabs.findIndex((t) => t.id === target.activeId);
      const next = target.tabs[(idx + dir + target.tabs.length) % target.tabs.length].id;
      const updated: Pane = { ...target, activeId: next };
      const panes: TaskPanes = {
        left: pane === "left" ? updated : old.left,
        right: pane === "right" ? updated : old.right ? { ...old.right } : null,
      };
      persist(taskId, panes, s.activePaneByTask[taskId] ?? "left");
      return { panesByTask: { ...s.panesByTask, [taskId]: panes } };
    });
  },

  toggleSplit: (taskId, taskName) => {
    set((s) => {
      const old = s.panesByTask[taskId];
      if (!old) return s;
      if (old.right) {
        const panes: TaskPanes = { left: old.left, right: null };
        const activePane: PaneId = "left";
        persist(taskId, panes, activePane);
        return {
          panesByTask: { ...s.panesByTask, [taskId]: panes },
          activePaneByTask: { ...s.activePaneByTask, [taskId]: activePane },
        };
      }
      // Split duplicates the active tab into the new right pane (reference
      // `splitActiveTab` semantics); ⌘D then adds a fresh conversation tab.
      const activeId = old.left.activeId ?? old.left.tabs[0].id;
      const tab =
        old.left.tabs.find((t) => t.id === activeId) ??
        ({ id: "conversation", kind: "conversation", title: taskName } as Tab);
      const panes: TaskPanes = {
        left: old.left,
        right: { tabs: [{ ...tab }], activeId: tab.id },
      };
      const activePane: PaneId = "right";
      persist(taskId, panes, activePane);
      return {
        panesByTask: { ...s.panesByTask, [taskId]: panes },
        activePaneByTask: { ...s.activePaneByTask, [taskId]: activePane },
      };
    });
  },

  setActivePane: (taskId, pane) => {
    set((s) => ({
      activePaneByTask: { ...s.activePaneByTask, [taskId]: pane },
    }));
    const panes = get().panesByTask[taskId];
    if (panes) persist(taskId, panes, pane);
  },

  setActiveTab: (taskId, pane, tabId) => {
    set((s) => {
      const old = s.panesByTask[taskId];
      const target = old?.[pane];
      if (!old || !target || !target.tabs.some((t) => t.id === tabId)) return s;
      const updated: Pane = { ...target, activeId: tabId };
      const panes: TaskPanes = {
        left: pane === "left" ? updated : old.left,
        right: pane === "right" ? updated : old.right ? { ...old.right } : null,
      };
      persist(taskId, panes, pane);
      return {
        panesByTask: { ...s.panesByTask, [taskId]: panes },
        activePaneByTask: { ...s.activePaneByTask, [taskId]: pane },
      };
    });
  },

  dropTask: (taskId) => {
    set((s) => {
      const panesByTask = { ...s.panesByTask };
      const activePaneByTask = { ...s.activePaneByTask };
      const loadedByTask = { ...s.loadedByTask };
      delete panesByTask[taskId];
      delete activePaneByTask[taskId];
      delete loadedByTask[taskId];
      return { panesByTask, activePaneByTask, loadedByTask };
    });
  },
}));

/** Drop local tab state when the backend deletes a task (mirrors the
 * backend's `task:<id>:tabs` view-state cleanup). */
export function wireTabsEvents(): () => void {
  let unlisten: (() => void) | undefined;
  void onAdeEvent((e) => {
    if (e.type === "task:deleted") useTabs.getState().dropTask(e.taskId);
  }).then((un) => {
    unlisten = un;
  });
  return () => unlisten?.();
}
