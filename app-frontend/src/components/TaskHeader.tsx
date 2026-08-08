// Task header (dogfood 2026-08-07): the top chrome stays identical across
// scopes — this is the task-scope row in the shell's header grid area.
// Hosts the lifecycle script launchers (E1-06) and the Changes toggle
// (E4-03), which the tab bars no longer carry.

import { useEffect, useState } from "react";
import { getProjectSettings } from "../lib/tauri";
import { openLifecycleScriptTab, openTaskChat } from "../lib/commands";
import { hint } from "../lib/useCommands";
import { useTabs } from "../store/tabs";
import { useUi } from "../store/ui";
import { useSidebar } from "../store/sidebar";
import { IconBranch, IconChat } from "./icons";

const SCRIPT_TYPES = ["setup", "run", "teardown"] as const;

export default function TaskHeader({
  taskId,
  projectId,
}: {
  taskId: string;
  projectId: string;
}) {
  const taskName = useSidebar(
    (s) =>
      (s.tasksByProject[projectId] ?? []).find((t) => t.id === taskId)?.name ??
      null,
  );
  const projectName = useSidebar(
    (s) => s.projects.find((p) => p.id === projectId)?.name ?? null,
  );
  const changesOpen = useUi((s) => s.changesOpen);
  const taskChatOpen = useUi((s) => s.taskChatOpen);
  // Scripts configured on the project (E1-06) — refetched per project open;
  // cheap DB read, so no caching layer is warranted.
  const [scripts, setScripts] = useState<Record<string, string> | null>(null);

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

  return (
    <header className="app-header task-header">
      <span className="task-header-title">
        {projectName ?? "Project"}
        <em> / </em>
        {taskName ?? "Task"}
      </span>
      <span className="app-header-actions">
        {scripts &&
          SCRIPT_TYPES.filter((k) => scripts[k]).map((k) => (
            <button
              key={k}
              className="script-run"
              title={`Run ${k} script`}
              onClick={() => {
                const pane = useTabs.getState().activePaneByTask[taskId] ?? "left";
                void openLifecycleScriptTab(taskId, pane, k);
              }}
            >
              {k}
            </button>
          ))}
        <button
          className={`project-action${changesOpen && !taskChatOpen ? " active" : ""}`}
          title={`Changes (${hint("toggle-changes") || "⌘⇧1"})`}
          onClick={() => {
            const ui = useUi.getState();
            if (!ui.changesOpen) {
              ui.setChangesOpen(true);
              ui.setTaskChatOpen(false);
            } else if (ui.taskChatOpen) {
              ui.setTaskChatOpen(false); // chat mode → changes mode
            } else {
              ui.setChangesOpen(false); // changes mode → close the sheet
            }
          }}
        >
          <IconBranch size={14} />
        </button>
        <button
          className={`project-action${taskChatOpen ? " active" : ""}`}
          title={`Agent chat (${hint("open-conversation") || "⌘⇧A"})`}
          onClick={() => openTaskChat()}
        >
          <IconChat size={14} />
        </button>
      </span>
    </header>
  );
}
