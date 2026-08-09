// Task view (E2-10, design_handoff_v2 5a/5b): the terminal-first task
// surface — 46px header (TaskHeader) over the task's pane(s), with the ⌘J
// script drawer as a bottom sheet. Pane content renders through the tab
// registry so new tab kinds drop in by registration.

import { useEffect, useState } from "react";
import Drawer from "./Drawer";
import TabBar from "./TabBar";
import TaskHeader from "./TaskHeader";
import { TAB_KINDS } from "../lib/tab-registry";
import type { CommandId } from "../lib/registry";
import { hint, runCommand } from "../lib/useCommands";
import { useScripts } from "../store/scripts";
import { useSidebar } from "../store/sidebar";
import { useTabs, type PaneId, type Pane } from "../store/tabs";
import { useUi } from "../store/ui";

export default function TaskView({ taskId }: { taskId: string }) {
  const panes = useTabs((s) => s.panesByTask[taskId]);
  const drawerOpen = useUi((s) => s.drawerOpen);

  useEffect(() => {
    void useTabs.getState().ensureTabs(taskId);
    // Lifecycle script state (5a launchers, 5b setup-failed label, 7b
    // drawer) hydrates from the backend's retained terminal entries.
    void useScripts.getState().hydrate(taskId);
  }, [taskId]);

  if (!panes) return null;

  // Every tab stays mounted — switching tabs only hides the view. Terminal
  // sessions survive unmounts anyway (session registry), but keeping them
  // mounted also preserves focus/scroll position cheaply.
  const renderPane = (pane: PaneId, state: Pane) => (
    <div className="pane-content">
      {state.tabs.length === 0 && <PaneEmpty taskId={taskId} />}
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
      <TaskHeader taskId={taskId} />
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
      {drawerOpen && <Drawer taskId={taskId} />}
    </div>
  );
}

/** 5b "nothing running": a stop-reason label over a 260px key list. Every
 * row is a real command — the view NEVER auto-spawns a shell. While setup
 * runs (7b) the whole thing yields to the dimmed "Waiting on setup" line. */
function PaneEmpty({ taskId }: { taskId: string }) {
  const task = useSidebar((s) => {
    for (const list of Object.values(s.tasksByProject)) {
      const t = list.find((x) => x.id === taskId);
      if (t) return t;
    }
    return null;
  });
  const setup = useScripts((s) => s.byTask[taskId]?.setup);
  const agentExitedAt = useScripts((s) => s.agentByTask[taskId]?.exitedAt ?? null);
  // Re-render hint text when keybindings change.
  useUi((s) => s.bindingsVersion);

  // Elapsed is derived on a slow tick, never stored — the display is
  // minute-coarse.
  const [, setTick] = useState(0);
  useEffect(() => {
    const t = setInterval(() => setTick((n) => n + 1), 30_000);
    return () => clearInterval(t);
  }, []);

  // 7b gate: while setup runs the agent will not start, so the key list
  // would lie — the pane shows the frame's dimmed one-liner instead.
  if (setup?.running) {
    return (
      <div className="tv-waiting">
        <div>
          <span className="tv-waiting-dot">●</span> Waiting on setup before
          starting…
        </div>
      </div>
    );
  }

  // A failed setup blocks the agent from starting (7b) — it outranks the
  // stopped/elapsed line.
  const setupFailed =
    setup !== undefined &&
    !setup.running &&
    setup.exitCode !== null &&
    setup.exitCode !== 0;
  // "stopped · elapsed" counts from the agent terminal's exit when this
  // session saw it (scripts store); statusChangedAt is only the fallback.
  const stoppedAt =
    agentExitedAt ??
    (task?.statusChangedAt ? Date.parse(task.statusChangedAt) : null);
  const label = setupFailed
    ? `setup failed · exit ${setup.exitCode}`
    : stoppedAt !== null
      ? `stopped · ${ago(stoppedAt)}`
      : "nothing running";

  const rows: { label: string; command: CommandId; fallback: string }[] = [
    { label: "Resume the agent", command: "resume-agent", fallback: "⌘T" },
    { label: "Split with a shell", command: "new-terminal-right-split", fallback: "⌘D" },
    { label: "New terminal", command: "new-terminal", fallback: "⌘⇧T" },
  ];

  return (
    <div className="tv-empty">
      <div className={`tv-empty-label${setupFailed ? " failed" : ""}`}>{label}</div>
      <div className="tv-empty-list">
        {rows.map((r) => (
          <button
            key={r.command}
            type="button"
            className="tv-empty-row"
            onClick={() => runCommand(r.command)}
          >
            <span>{r.label}</span>
            <span className="tv-empty-key">{hint(r.command) || r.fallback}</span>
          </button>
        ))}
      </div>
    </div>
  );
}

/** Relative time, coarse: now / Nm / Nh / Nd / Nw (mirrors Nav.tsx). */
function ago(ts: number): string {
  const s = Math.max(0, (Date.now() - ts) / 1000);
  if (s < 90) return "now";
  const m = s / 60;
  if (m < 60) return `${Math.round(m)}m`;
  const h = m / 60;
  if (h < 24) return `${Math.round(h)}h`;
  const d = h / 24;
  if (d < 7) return `${Math.round(d)}d`;
  return `${Math.round(d / 7)}w`;
}
