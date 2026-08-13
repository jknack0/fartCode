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
import FileEditorView from "../components/FileEditorView";
import FileTreeView from "../components/FileTreeView";
import DiffView from "../components/DiffView";
import TerminalView from "../components/TerminalView";
import type { PaneId } from "../store/tabs";

/** A tab open in a task pane; `id` is unique within the pane. */
export interface Tab {
  id: string;
  kind: TabKind;
  title: string;
}

// Lifecycle-script tabs were removed (design_handoff_v2 7b): script
// terminals live in the ⌘J drawer (store/scripts.ts), never in the tab
// bar. Persisted lifecycle tabs are dropped by sanitizePane because the
// kind is no longer registered.
export type TabKind = "terminal" | "conversation" | "diff" | "files" | "file-editor";

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
    render: ({ taskId, tab, pane, active }) => (
      <DiffView tabId={tab.id} title={tab.title} taskId={taskId} pane={pane} active={active} />
    ),
  },

  "file-editor": {
    label: "Editor",
    glyph: "ED",
    // The tab id encodes the file (`edit:<workspaceId>:<path>`); restored
    // tabs re-parse and refetch content (unsaved text is E5-03's job).
    render: ({ tab, active }) => <FileEditorView tabId={tab.id} active={active} />,
  },

  files: {
    label: "Files",
    glyph: "FS",
    // The tab id encodes the workspace (`files:<workspaceId>`), so a tab
    // restored from view-state re-resolves without a sidecar (E5-01).
    render: ({ taskId, tab, active }) => (
      <FileTreeView
        taskId={taskId}
        workspaceId={tab.id.slice("files:".length)}
        active={active}
      />
    ),
  },

};

export function isTabKind(kind: string): kind is TabKind {
  return kind in TAB_KINDS;
}
