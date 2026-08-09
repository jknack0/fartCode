// ⌘J drawer (design_handoff_v2 7b): a 210px bottom sheet on the task view
// holding the three lifecycle script terminals. Tabs are the scripts; a
// failed script's tab reads `setup ✗ exit 1` with a red underline. `r`
// reruns the active script while the drawer chrome holds focus; logs
// persist per task (the backend retains lifecycle entries and replays the
// tail on reattach).

import { useEffect, useRef } from "react";
import TerminalView from "./TerminalView";
import { hint, runCommand } from "../lib/useCommands";
import { SCRIPT_TYPES, useScripts } from "../store/scripts";
import { useUi } from "../store/ui";

export default function Drawer({ taskId }: { taskId: string }) {
  const active = useUi((s) => s.drawerScript);
  const setDrawerScript = useUi((s) => s.setDrawerScript);
  const runs = useScripts((s) => s.byTask[taskId]);
  // Re-render hint text when keybindings change.
  useUi((s) => s.bindingsVersion);
  const rootRef = useRef<HTMLDivElement>(null);

  const activeRun = runs?.[active];
  const activeTerminalId = activeRun?.terminalId ?? null;

  // The drawer chrome takes focus on open and after tab/terminal flips —
  // TerminalView focuses xterm on mount (child effects run first), so the
  // parent effect wins and `r` lands on the chrome, not in the script's
  // stdin. Clicking into the terminal hands the keyboard back to it.
  useEffect(() => {
    rootRef.current?.focus();
  }, [active, activeTerminalId]);

  const rerun = () => {
    void useScripts
      .getState()
      .rerun(taskId, active)
      .catch((e) => console.error("script rerun failed", e));
  };

  const closeHint = (hint("toggle-drawer") || "⌘J").toLowerCase();

  return (
    <div
      ref={rootRef}
      className="drawer"
      tabIndex={-1}
      onKeyDown={(e) => {
        // `r` reruns the active script — but keys typed into the script's
        // terminal (or any input) stay where they were typed.
        if (e.key !== "r" || e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
        const t = e.target as HTMLElement;
        if (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable) return;
        e.preventDefault();
        rerun();
      }}
    >
      <div className="drawer-strip">
        <div className="drawer-tabs">
          {SCRIPT_TYPES.map((k) => {
            const run = runs?.[k];
            const failed =
              !run?.running &&
              run?.exitCode !== null &&
              run?.exitCode !== undefined &&
              run.exitCode !== 0;
            return (
              <button
                key={k}
                type="button"
                className={`drawer-tab${k === active ? " active" : ""}${failed ? " failed" : ""}`}
                onClick={() => setDrawerScript(k)}
              >
                {k}
                {failed ? ` ✗ exit ${run.exitCode}` : ""}
              </button>
            );
          })}
        </div>
        <div className="drawer-keys">
          <button type="button" className="drawer-key" onClick={rerun}>
            r rerun
          </button>
          <span>·</span>
          <button
            type="button"
            className="drawer-key"
            onClick={() => runCommand("toggle-drawer")}
          >
            {closeHint} close
          </button>
        </div>
      </div>
      <div className="drawer-body">
        {activeTerminalId ? (
          <TerminalView terminalId={activeTerminalId} active />
        ) : (
          <div className="drawer-empty">not run yet · r runs it</div>
        )}
      </div>
    </div>
  );
}
