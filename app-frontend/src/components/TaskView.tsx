// Task view (E2-10): the task's pane(s) + tab bar(s). Pane content is
// rendered through the tab registry so new tab kinds drop in by registration.
// Chrome lives in the app header row (TaskHeader) — the tab bars are pure
// tab switching, identical in shape to the project scope's header.

import { useEffect } from "react";
import TabBar from "./TabBar";
import { TAB_KINDS } from "../lib/tab-registry";
import { hint } from "../lib/useCommands";
import { useTabs, type PaneId, type Pane } from "../store/tabs";

export default function TaskView({ taskId }: { taskId: string }) {
  const panes = useTabs((s) => s.panesByTask[taskId]);

  useEffect(() => {
    void useTabs.getState().ensureTabs(taskId);
  }, [taskId]);

  if (!panes) return null;

  // Every tab stays mounted — switching tabs only hides the view. Terminal
  // sessions survive unmounts anyway (session registry), but keeping them
  // mounted also preserves focus/scroll position cheaply.
  const renderPane = (pane: PaneId, state: Pane) => (
    <div className="pane-content">
      {state.tabs.length === 0 && (
        <div className="pane-empty">
          <p className="muted">Nothing running in this pane.</p>
          <p className="muted">
            <span className="kbd-hint">{hint("new-terminal") || "⌘T"}</span>{" "}
            opens a terminal,{" "}
            <span className="kbd-hint">
              {hint("new-terminal-right-split") || "⌘D"}
            </span>{" "}
            splits one.
          </p>
        </div>
      )}
      {state.tabs.map((tab) => {
        const def = TAB_KINDS[tab.kind];
        const isActive = tab.id === state.activeId;
        return (
          <div
            key={tab.id}
            className="tab-content"
            style={isActive ? undefined : { display: "none" }}
          >
            {def.render({ taskId, tab, pane, active: isActive })}
          </div>
        );
      })}
    </div>
  );

  // ADR-0033: one agent terminal per task — when that's all there is, the
  // pane IS the task and a lone tab chip is pure noise (the header already
  // names project/task). The bar appears once there's something to switch
  // between: a second tab, or a split (whose active-pane tint is the only
  // focus affordance).
  const showLeftTabBar = Boolean(panes.right) || panes.left.tabs.length > 1;

  return (
    <div className="task-view">
      <div className="task-panes">
        <section className="pane">
          {showLeftTabBar && <TabBar taskId={taskId} pane="left" />}
          {renderPane("left", panes.left)}
        </section>
        {panes.right && (
          <section className="pane">
            <TabBar taskId={taskId} pane="right" />
            {renderPane("right", panes.right)}
          </section>
        )}
      </div>
    </div>
  );
}
