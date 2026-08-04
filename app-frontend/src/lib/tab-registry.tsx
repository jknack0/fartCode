// Tab registry (E2-10): the set of tab kinds a task pane can show, and the
// component that renders each. Adding a tab kind is a one-entry change:
// extend `TabKind` AND add an entry to `TAB_KINDS` — the tab bar, shortcuts,
// and view-state persistence pick it up automatically.
//
// Later tickets register their kinds here:
//   diff        — E4-04 (diff renderer)
//   browser     — E6-01 (per-task browser tab, ⌘⇧B)
//   file-editor — E5-02 (editor tabs, ⌘S/⌘⇧S)
import type { ReactNode } from "react";
import ConversationView from "../components/ConversationView";
import type { PaneId } from "../store/tabs";

/** A tab open in a task pane; `id` is unique within the pane. */
export interface Tab {
  id: string;
  kind: TabKind;
  title: string;
}

export type TabKind = "conversation";

export interface TabRenderProps {
  taskId: string;
  tab: Tab;
  pane: PaneId;
}

export interface TabKindDef {
  label: string;
  render: (props: TabRenderProps) => ReactNode;
}

export const TAB_KINDS: Record<TabKind, TabKindDef> = {
  conversation: {
    label: "Conversation",
    render: ({ taskId }) => <ConversationView taskId={taskId} />,
  },
};

export function isTabKind(kind: string): kind is TabKind {
  return kind in TAB_KINDS;
}
