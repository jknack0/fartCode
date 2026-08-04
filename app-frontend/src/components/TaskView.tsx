// Task view (E2-10): the task's pane(s) + tab bar(s). Pane content is
// rendered through the tab registry so new tab kinds drop in by registration.
import { useEffect } from "react";
import TabBar from "./TabBar";
import { TAB_KINDS } from "../lib/tab-registry";
import { useTabs, type PaneId, type Pane } from "../store/tabs";
import { useSidebar } from "../store/sidebar";

export default function TaskView({
  taskId,
  projectId,
}: {
  taskId: string;
  projectId: string;
}) {
  const taskName = useSidebar(
    (s) =>
      (s.tasksByProject[projectId] ?? []).find((t) => t.id === taskId)?.name ??
      "Task",
  );
  const panes = useTabs((s) => s.panesByTask[taskId]);

  useEffect(() => {
    void useTabs.getState().ensureTabs(taskId, taskName);
  }, [taskId, taskName]);

  if (!panes) return null;

  const renderPane = (pane: PaneId, state: Pane) => {
    const tab =
      state.tabs.find((t) => t.id === state.activeId) ?? state.tabs[0];
    if (!tab) return null;
    const def = TAB_KINDS[tab.kind];
    return (
      <div className="pane-content">
        {def.render({ taskId, tab, pane })}
      </div>
    );
  };

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
