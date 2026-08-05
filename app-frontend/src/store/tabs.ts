// Per-task tab state (E2-10): each task has a left pane (always) and an
// optional right pane (⌘\ split). Every pane owns a tab list + active tab.
// Tab state persists under `view-state:task:<id>:tabs` — the exact key the
// backend tears down with the task (E2-09) and prunes on boot (E1-08).
//
// Every tab is a terminal (chat surfaces were removed). Terminal tabs
// survive restarts: their ids are PTY ids, and on restore the PTY is
// respawned for the same task. With the project's tmux setting on the
// respawn REATTACHES the surviving tmux session (ADR-0025/0028), and any
// survivor the persisted tabs don't cover gets an extra tab so every
// existing terminal shows on reopen. Future kinds (diff, browser,
// file-editor) register in `lib/tab-registry.tsx` and flow through the
// same machinery.
import { create } from "zustand";
import {
  getViewState,
  onAdeEvent,
  setViewState,
  terminalOpen,
  terminalSurviving,
} from "../lib/tauri";
import { killTerminal } from "../lib/terminals";
import { isTabKind, type Tab } from "../lib/tab-registry";

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

  ensureTabs: (taskId: string) => Promise<void>;
  addTab: (taskId: string, pane: PaneId, tab: Tab) => void;
  closeTab: (taskId: string, pane: PaneId, tabId: string) => void;
  jumpToTab: (taskId: string, pane: PaneId, n: number) => void;
  cycleTab: (taskId: string, pane: PaneId, dir: 1 | -1) => void;
  toggleSplit: (taskId: string) => Promise<void>;
  setActivePane: (taskId: string, pane: PaneId) => void;
  setActiveTab: (taskId: string, pane: PaneId, tabId: string) => void;
  dropTask: (taskId: string) => void;
}

/// Validates a persisted pane: registry-known kinds only, at least one tab,
/// active id present. Terminal tabs are kept — their PTYs are respawned on
/// restore (restart-survival contract for the terminal-only task view).
function sanitizePane(pane: Pane | undefined): Pane | null {
  if (!pane) return null;
  const tabs = pane.tabs.filter(
    (t): t is Tab =>
      typeof t?.id === "string" &&
      typeof t?.title === "string" &&
      typeof t?.kind === "string" &&
      isTabKind(t.kind),
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

  ensureTabs: async (taskId) => {
    if (get().loadedByTask[taskId] || hydrating.has(taskId)) return;
    hydrating.add(taskId);
    try {
      const saved = (await getViewState(
        `view-state:task:${taskId}:tabs`,
      )) as SavedTabs | null;

      // Reconcile persisted tabs: respawn every terminal's PTY. With tmux
      // durability each respawn reattaches one surviving session
      // (ADR-0028 slot reuse); without it the old shells died with the
      // previous process and the respawn is a fresh shell. Conversation
      // tabs restore as-is — their id IS the conversation id (a DB row),
      // and the view rehydrates the transcript from acp_history (#33).
      const reconcile = async (pane: Pane | undefined): Promise<Pane> => {
        const clean = sanitizePane(pane);
        if (!clean) return { tabs: [], activeId: null };
        const kept: Tab[] = [];
        for (const t of clean.tabs) {
          if (t.kind !== "terminal") {
            kept.push(t);
            continue;
          }
          try {
            const freshId = await terminalOpen(taskId, 24, 80);
            kept.push({ ...t, id: freshId });
          } catch (e) {
            console.warn("terminal respawn failed on restore:", e);
          }
        }
        if (kept.length === 0) return { tabs: [], activeId: null };
        const activeId = kept.some((t) => t.id === clean.activeId)
          ? clean.activeId
          : kept[0].id;
        return { tabs: kept, activeId };
      };

      let left = await reconcile(saved?.left);
      let right: Pane | null = saved?.right ? await reconcile(saved.right) : null;
      if (right && right.tabs.length === 0) right = null;

      // Show existing terminals that survived a crash/close (ADR-0028):
      // every live session this process does not already cover gets a tab.
      // Each open reattaches the next detached survivor (backend slot
      // policy), so nothing is spawned fresh while a survivor exists.
      try {
        const surviving = await terminalSurviving(taskId);
        for (let i = 0; i < surviving; i++) {
          const id = await terminalOpen(taskId, 24, 80);
          left = {
            tabs: [...left.tabs, { id, kind: "terminal", title: "Terminal" }],
            activeId: left.activeId ?? id,
          };
        }
      } catch (e) {
        console.warn("survivor restore failed:", e);
      }
      if (left.tabs.length === 0) {
        // Terminal-first: a task's default surface is a shell in its
        // worktree; extra terminals are summoned explicitly (⌘T / ⌘D),
        // never auto-opened. (E2-12: work inside ade.)
        try {
          const terminalId = await terminalOpen(taskId, 24, 80);
          left = {
            tabs: [{ id: terminalId, kind: "terminal", title: "Terminal" }],
            activeId: terminalId,
          };
        } catch (e) {
          console.warn("terminal_open failed on task open:", e);
          left = { tabs: [], activeId: null };
        }
      }
      const panes: TaskPanes = { left, right };
      // Persist the respawned terminal ids immediately so the stored state
      // never carries a previous-process PTY id past first restore. The
      // saved activePane is only meaningful when the right pane survived.
      const activePane: PaneId = right ? saved?.activePane ?? "left" : "left";
      persist(taskId, panes, activePane);
      set((s) => ({
        panesByTask: { ...s.panesByTask, [taskId]: panes },
        activePaneByTask: { ...s.activePaneByTask, [taskId]: activePane },
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
      const target: Pane = { tabs: remaining, activeId };
      const panes: TaskPanes = {
        left: pane === "left" ? target : old.left,
        right: pane === "right" ? target : old.right ? { ...old.right } : null,
      };
      persist(taskId, panes, activePane);
      return { panesByTask: { ...s.panesByTask, [taskId]: panes } };
    });

    // Terminal tabs own their PTY: closing the LAST reference kills the
    // shell. Tab flips only hide the view, which keeps the shell (and its
    // scrollback) alive.
    if (closedTab?.kind === "terminal") {
      const after = get().panesByTask[taskId];
      const stillReferenced = [after?.left.tabs ?? [], after?.right?.tabs ?? []]
        .flat()
        .some((t) => t.id === tabId);
      if (!stillReferenced) killTerminal(tabId);
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

  toggleSplit: async (taskId) => {
    let old = get().panesByTask[taskId];
    if (!old) return;
    if (old.right) {
      // Collapse: terminals referenced ONLY by the right pane die with it
      // (split-spawned shells are never shared with the left pane).
      const rightOnlyTerminals = old.right.tabs
        .filter((t) => t.kind === "terminal" && !old!.left.tabs.some((l) => l.id === t.id))
        .map((t) => t.id);
      const panes: TaskPanes = { left: old.left, right: null };
      const activePane: PaneId = "left";
      persist(taskId, panes, activePane);
      set((s) => ({
        panesByTask: { ...s.panesByTask, [taskId]: panes },
        activePaneByTask: { ...s.activePaneByTask, [taskId]: activePane },
      }));
      for (const id of rightOnlyTerminals) killTerminal(id);
      return;
    }
    // Split spawns a fresh shell in the new right pane — one PTY drives one
    // xterm surface, so terminals are never shared across panes. An empty
    // left pane (terminal_open failed at boot) splits into an empty pane.
    const activeId = old.left.activeId ?? old.left.tabs[0]?.id ?? null;
    let rightTab: Tab | null = null;
    if (activeId) {
      try {
        const freshId = await terminalOpen(taskId, 24, 80);
        rightTab = { id: freshId, kind: "terminal", title: "Terminal" };
      } catch (e) {
        console.warn("terminal open failed on split:", e);
        return;
      }
      // The spawn awaited — state may have moved underneath us.
      old = get().panesByTask[taskId];
      if (!old || old.right) return;
    }
    const panes: TaskPanes = {
      left: old.left,
      right: {
        tabs: rightTab ? [rightTab] : [],
        activeId: rightTab ? rightTab.id : null,
      },
    };
    const activePane: PaneId = "right";
    persist(taskId, panes, activePane);
    set((s) => ({
      panesByTask: { ...s.panesByTask, [taskId]: panes },
      activePaneByTask: { ...s.activePaneByTask, [taskId]: activePane },
    }));
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
    // Kill the task's terminal sessions (backend `close_task` already killed
    // the PTYs on task deletion; this frees xterm state + listeners).
    const panes = get().panesByTask[taskId];
    const terminalIds = [...(panes?.left.tabs ?? []), ...(panes?.right?.tabs ?? [])]
      .filter((t) => t.kind === "terminal")
      .map((t) => t.id);
    set((s) => {
      const panesByTask = { ...s.panesByTask };
      const activePaneByTask = { ...s.activePaneByTask };
      const loadedByTask = { ...s.loadedByTask };
      delete panesByTask[taskId];
      delete activePaneByTask[taskId];
      delete loadedByTask[taskId];
      return { panesByTask, activePaneByTask, loadedByTask };
    });
    for (const id of terminalIds) killTerminal(id);
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
