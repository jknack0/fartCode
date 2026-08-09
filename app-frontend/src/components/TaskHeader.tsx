// Task header (design_handoff_v2 5a): the 46px terminal-first header row —
// breadcrumb + title + agent dot on the left; lifecycle script launchers
// and the Changes toggle on the right, all mono 11px. A launcher opens the
// ⌘J drawer on its script's tab (starting the script only when it has
// never run); its suffix carries the last run's outcome (✓ / ✗).

import { useEffect, useState } from "react";
import { getProjectSettings } from "../lib/tauri";
import { openLifecycleScript } from "../lib/commands";
import { hint, runCommand } from "../lib/useCommands";
import { SCRIPT_TYPES, useScripts, type ScriptType } from "../store/scripts";
import { useSidebar } from "../store/sidebar";
import { useUi } from "../store/ui";

export default function TaskHeader({ taskId }: { taskId: string }) {
  const projectId = useSidebar((s) => s.selectedProjectId);
  const projectName = useSidebar(
    (s) => s.projects.find((p) => p.id === s.selectedProjectId)?.name ?? null,
  );
  const task = useSidebar((s) =>
    s.selectedProjectId
      ? ((s.tasksByProject[s.selectedProjectId] ?? []).find((t) => t.id === taskId) ?? null)
      : null,
  );
  const runs = useScripts((s) => s.byTask[taskId]);
  const agentRunning = useScripts((s) => s.agentByTask[taskId]?.running ?? false);
  // Re-render hint text when keybindings change.
  useUi((s) => s.bindingsVersion);

  // Scripts configured on the project (E1-06) — only configured scripts get
  // a launcher. Cheap DB read, refetched per project.
  const [configured, setConfigured] = useState<ScriptType[]>([]);
  useEffect(() => {
    if (!projectId) return;
    let live = true;
    getProjectSettings(projectId)
      .then((s) => {
        if (!live) return;
        setConfigured(SCRIPT_TYPES.filter((k) => Boolean(s.scripts?.[k]?.trim())));
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [projectId]);

  // Agent dot: the AGENT, not the lane — a live agent terminal (scripts
  // store, kept fresh by hydrate/spawn/exit) = filled amber pulse; review =
  // hollow amber needs you; anything else = idle (--dot-idle). task.status
  // never says "running" here because it never changes while the agent works.
  const dotClass = agentRunning
    ? "status-dot status-in_progress"
    : task?.status === "review"
      ? "status-dot status-needs-you"
      : "status-dot tv-dot-idle";

  return (
    <header className="tv-header">
      <div className="tv-header-id">
        <span className="tv-crumb">{projectName ?? "project"} /</span>
        <span className="tv-title">{task?.name ?? "Task"}</span>
        <span className={dotClass} />
      </div>
      <div className="tv-header-actions">
        {configured.map((k) => {
          const run = runs?.[k];
          const running = run?.running ?? false;
          const failed =
            !running && run?.exitCode !== null && run?.exitCode !== undefined && run.exitCode !== 0;
          const ok = !running && run?.exitCode === 0;
          return (
            <button
              key={k}
              type="button"
              className={`tv-action${running ? " running" : ""}${failed ? " failed" : ""}`}
              title={`${k} script — logs in the drawer (${hint("toggle-drawer") || "⌘J"})`}
              onClick={() => openLifecycleScript(taskId, k)}
            >
              {k}
              {ok ? " ✓" : failed ? " ✗" : ""}
            </button>
          );
        })}
        <button
          type="button"
          className="tv-action"
          title="Toggle changes panel"
          onClick={() => runCommand("toggle-changes")}
        >
          {`${hint("toggle-changes") || "⌘⇧1"} changes`}
        </button>
      </div>
    </header>
  );
}
