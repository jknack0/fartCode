// Task view (E2-10): the task's pane(s) + tab bar(s). Pane content is
// rendered through the tab registry so new tab kinds drop in by registration.
import { useEffect } from "react";
import TabBar from "./TabBar";
import { TAB_KINDS } from "../lib/tab-registry";
import { useTabs, type PaneId, type Pane } from "../store/tabs";

export default function TaskView({
  taskId,
}: {
  taskId: string;
  projectId: string;
}) {
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

  return (
    <div className="task-view">
      <div className="task-panes">
        <section className="pane">
          <TabBar taskId={taskId} pane="left" />
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
