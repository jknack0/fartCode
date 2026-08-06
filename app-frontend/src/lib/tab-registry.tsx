// Tab registry (E2-10): the set of tab kinds a task pane can show, and the
// component that renders each. Adding a tab kind is a one-entry change:
// extend `TabKind` AND add an entry to `TAB_KINDS` — the tab bar, shortcuts,
// and view-state persistence pick it up automatically.
//
// Later tickets may register their kinds here:
//   diff        — E4-04 (diff renderer)
//   browser     — E6-01 (per-task browser tab, ⌘⇧B)
//   file-editor — E5-02 (editor tabs, ⌘S/⌘⇧S)
// Registered: terminal (E2-12) and conversation (E2-11-6 structured chat —
// the tab id IS the conversation id, a DB row that survives restarts).
import type { ReactNode } from "react";
import ConversationView from "../components/ConversationView";
import DiffView from "../components/DiffView";
import TerminalView from "../components/TerminalView";
import type { PaneId } from "../store/tabs";

/** A tab open in a task pane; `id` is unique within the pane. */
export interface Tab {
  id: string;
  kind: TabKind;
  title: string;
}

export type TabKind = "terminal" | "conversation" | "diff" | "lifecycle-script";

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
  /** Short glyph shown in the tab before the title (mono voice). */
  glyph: string;
  render: (props: TabRenderProps) => ReactNode;
}

export const TAB_KINDS: Record<TabKind, TabKindDef> = {

  terminal: {
    label: "Terminal",
    glyph: "TTY",
    // The tab id IS the PTY id (minted by terminal_open); on restart the
    // tabs store respawns the PTY and rewrites the tab id.
    render: ({ tab, active }) => (
      <TerminalView terminalId={tab.id} active={active} />
    ),
  },

  conversation: {
    label: "Agent",
    glyph: "ACP",
    // The tab id IS the conversation id; the transcript lives in the
    // conversations store and survives tab flips and restarts (#33).
    render: ({ taskId, tab, active }) => (
      <ConversationView conversationId={tab.id} ownerKey={taskId} active={active} />
    ),
  },

  diff: {
    label: "Diff",
    glyph: "DIFF",
    // The tab id encodes the diff params (`diff:<side>:<workspaceId>:<path>`);
    // the diffs store holds the payload (#44, E4-04).
    render: ({ taskId, tab, active }) => (
      <DiffView tabId={tab.id} title={tab.title} taskId={taskId} active={active} />
    ),
  },

  "lifecycle-script": {
    label: "Script",
    glyph: "SCR",
    // The tab id IS the PTY id minted by terminal_open_lifecycle. The
    // terminal runs the script with the ADE_* env contract; when the script
    // exits the tab stays (the backend retains the entry) and the tail is
    // replayed on reattach (E1-06).
    render: ({ tab, active }) => (
      <TerminalView terminalId={tab.id} active={active} />
    ),
  },
};

export function isTabKind(kind: string): kind is TabKind {
  return kind in TAB_KINDS;
}

/** Tab title for a lifecycle script terminal (E1-06). The script type is
 * the backend's `scriptType` (`setup`/`run`/`teardown`). */
const SCRIPT_TITLES: Record<string, string> = {
  setup: "Setup script",
  run: "Run script",
  teardown: "Teardown script",
};

export function scriptTabTitle(scriptType: string): string {
  return SCRIPT_TITLES[scriptType] ?? "Script";
}
