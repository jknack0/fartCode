// Task view (E2-10): the task's pane(s) + tab bar(s). Pane content is
// rendered through the tab registry so new tab kinds drop in by registration.
// The Changes toggle (E4-03) lives at the right edge of the top-most bar —
// the right pane's when split, else the single pane's.
import { useEffect } from "react";
import TabBar from "./TabBar";
import { IconBranch } from "./icons";
import { TAB_KINDS } from "../lib/tab-registry";
import { hint } from "../lib/useCommands";
import { useUi } from "../store/ui";
import { useTabs, type PaneId, type Pane } from "../store/tabs";

export default function TaskView({
  taskId,
}: {
  taskId: string;
  projectId: string;
}) {
  const panes = useTabs((s) => s.panesByTask[taskId]);
  const changesOpen = useUi((s) => s.changesOpen);
  const setChangesOpen = useUi((s) => s.setChangesOpen);

  useEffect(() => {
    void useTabs.getState().ensureTabs(taskId);
  }, [taskId]);

  if (!panes) return null;

  const changesToggle = (
    <button
      className={`changes-toggle${changesOpen ? " active" : ""}`}
      title={`Changes (${hint("toggle-changes") || "⌘⇧1"})`}
      onClick={() => setChangesOpen(!changesOpen)}
    >
      <IconBranch size={12} />
    </button>
  );
  const barPane: PaneId = panes.right ? "right" : "left";

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
          <TabBar
            taskId={taskId}
            pane="left"
            trailing={barPane === "left" ? changesToggle : undefined}
          />
          {renderPane("left", panes.left)}
        </section>
        {panes.right && (
          <section className="pane">
            <TabBar
              taskId={taskId}
              pane="right"
              trailing={barPane === "right" ? changesToggle : undefined}
            />
            {renderPane("right", panes.right)}
          </section>
        )}
      </div>
    </div>
  );
}
