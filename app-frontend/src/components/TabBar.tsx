// Tab bar (E2-10): one per task pane. Tabs are terminals; click to
// activate, × to close (⌘W), ⌘T adds another. `trailing` renders at the
// bar's right edge (the changes toggle lives there — E4-03).
import type { ReactNode } from "react";
import { useTabs, type PaneId } from "../store/tabs";
import { TAB_KINDS } from "../lib/tab-registry";
import { IconClose } from "./icons";

export default function TabBar({
  taskId,
  pane,
  trailing,
}: {
  taskId: string;
  pane: PaneId;
  trailing?: ReactNode;
}) {
  const panes = useTabs((s) => s.panesByTask[taskId]);
  const activePane = useTabs((s) => s.activePaneByTask[taskId] ?? "left");
  const paneState = panes?.[pane];
  if (!paneState) return null;

  const activate = (tabId: string) => {
    useTabs.getState().setActiveTab(taskId, pane, tabId);
  };

  return (
    <div className={`tab-bar${activePane === pane ? " active" : ""}`}>
      {paneState.tabs.map((tab) => (
        <div
          key={tab.id}
          className={`tab${tab.id === paneState.activeId ? " active" : ""}`}
          onClick={() => activate(tab.id)}
          title={`${tab.title} — ${tab.id === paneState.activeId ? "⌘W to close" : "click to activate"}`}
        >
          <span className="tab-kind">{TAB_KINDS[tab.kind].glyph}</span>
          <span className="tab-title">{tab.title}</span>
          <button
            className="tab-close"
            title="Close tab (⌘W)"
            onClick={(e) => {
              e.stopPropagation();
              useTabs.getState().closeTab(taskId, pane, tab.id);
            }}
          >
            <IconClose size={9} />
          </button>
        </div>
      ))}
      {trailing && <div className="tab-bar-trailing">{trailing}</div>}
    </div>
  );
}
