// Task view (E2-10): the task's pane(s) + tab bar(s). Pane content is
// rendered through the tab registry so new tab kinds drop in by registration.
// The Changes toggle (E4-03) lives at the right edge of the top-most bar —
// the right pane's when split, else the single pane's. Lifecycle script
// launchers (E1-06) sit next to it for every configured script.
import { useEffect, useState } from "react";
import TabBar from "./TabBar";
import { IconBranch } from "./icons";
import { getProjectSettings } from "../lib/tauri";
import { openLifecycleScriptTab } from "../lib/commands";
import { TAB_KINDS } from "../lib/tab-registry";
import { hint } from "../lib/useCommands";
import { useUi } from "../store/ui";
import { useTabs, type PaneId, type Pane } from "../store/tabs";

const SCRIPT_TYPES = ["setup", "run", "teardown"] as const;

export default function TaskView({
  taskId,
  projectId,
}: {
  taskId: string;
  projectId: string;
}) {
  const panes = useTabs((s) => s.panesByTask[taskId]);
  const changesOpen = useUi((s) => s.changesOpen);
  const setChangesOpen = useUi((s) => s.setChangesOpen);
  // Scripts configured on the project (E1-06) — refetched per project open;
  // cheap DB read, so no caching layer is warranted.
  const [scripts, setScripts] = useState<Record<string, string> | null>(null);

  useEffect(() => {
    void useTabs.getState().ensureTabs(taskId);
  }, [taskId]);

  useEffect(() => {
    let live = true;
    getProjectSettings(projectId)
      .then((s) => {
        if (!live) return;
        const configured: Record<string, string> = {};
        for (const k of SCRIPT_TYPES) {
          const text = s.scripts?.[k]?.trim();
          if (text) configured[k] = text;
        }
        setScripts(configured);
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [projectId]);

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
  const trailing = (
    <div className="tab-bar-actions">
      {scripts &&
        SCRIPT_TYPES.filter((k) => scripts[k]).map((k) => (
          <button
            key={k}
            className="script-run"
            title={`Run ${k} script`}
            onClick={() => void openLifecycleScriptTab(taskId, barPane, k)}
          >
            {k}
          </button>
        ))}
      {changesToggle}
    </div>
  );

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
            trailing={barPane === "left" ? trailing : undefined}
          />
          {renderPane("left", panes.left)}
        </section>
        {panes.right && (
          <section className="pane">
            <TabBar
              taskId={taskId}
              pane="right"
              trailing={barPane === "right" ? trailing : undefined}
            />
            {renderPane("right", panes.right)}
          </section>
        )}
      </div>
    </div>
  );
}
