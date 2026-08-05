// Tab registry (E2-10): the set of tab kinds a task pane can show, and the
// component that renders each. Adding a tab kind is a one-entry change:
// extend `TabKind` AND add an entry to `TAB_KINDS` — the tab bar, shortcuts,
// and view-state persistence pick it up automatically.
//
// Later tickets may register their kinds here:
//   diff        — E4-04 (diff renderer)
//   browser     — E6-01 (per-task browser tab, ⌘⇧B)
//   file-editor — E5-02 (editor tabs, ⌘S/⌘⇧S)
// Registered now: terminal only — chat surfaces were removed; everything
// that opens is a terminal (E2-12).
import type { ReactNode } from "react";
import TerminalView from "../components/TerminalView";
import type { PaneId } from "../store/tabs";

/** A tab open in a task pane; `id` is unique within the pane. */
export interface Tab {
  id: string;
  kind: TabKind;
  title: string;
}

export type TabKind = "terminal";

export interface TabRenderProps {
  taskId: string;
  tab: Tab;
  pane: PaneId;
  /** The pane's active tab gets keyboard focus and is visible; inactive
   * tabs stay mounted (their session survives a tab switch) but hidden. */
  active: boolean;
}

export interface TabKindDef {
  label: string;
  render: (props: TabRenderProps) => ReactNode;
}

export const TAB_KINDS: Record<TabKind, TabKindDef> = {
  terminal: {
    label: "Terminal",
    // The tab id IS the PTY id (minted by terminal_open); on restart the
    // tabs store respawns the PTY and rewrites the tab id.
    render: ({ tab, active }) => (
      <TerminalView terminalId={tab.id} active={active} />
    ),
  },
};

export function isTabKind(kind: string): kind is TabKind {
  return kind in TAB_KINDS;
}
