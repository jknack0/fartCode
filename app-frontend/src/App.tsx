// ade app shell (E1-04/E2-08/E2-10/E14-01): sidebar + task view with tabs.
// All keyboard shortcuts are registry commands (lib/commands.ts); the single
// dispatch listener is installed by useCommands().

import { useEffect } from "react";
import CommandPalette from "./components/CommandPalette";
import Modals from "./components/Modals";
import Onboarding from "./components/Onboarding";
import ResourceMonitor from "./components/ResourceMonitor";
import Sidebar from "./components/Sidebar";
import TaskView from "./components/TaskView";
import { useCommands } from "./lib/useCommands";
import { useSidebar, wireSidebarEvents } from "./store/sidebar";
import { wireTabsEvents } from "./store/tabs";

function App() {
  const load = useSidebar((s) => s.load);
  const selectedProjectId = useSidebar((s) => s.selectedProjectId);
  const selectedTaskId = useSidebar((s) => s.selectedTaskId);
  const error = useSidebar((s) => s.error);

  useEffect(() => {
    load().catch(() => {});
    const unlisten = wireSidebarEvents();
    const unlistenTabs = wireTabsEvents();
    return () => {
      unlisten();
      unlistenTabs();
    };
  }, [load]);

  useCommands();

  return (
    <main className="shell">
      <Sidebar />
      <Onboarding />
      <CommandPalette />
      <Modals />
      <ResourceMonitor />
      <section className="main">
        {error && <p className="error">{error}</p>}
        {selectedTaskId && selectedProjectId ? (
          <TaskView taskId={selectedTaskId} projectId={selectedProjectId} />
        ) : selectedProjectId ? (
          <div className="placeholder">
            <h1>Project chat</h1>
            <p className="muted">Coming in Phase 1 — project-level agent chat.</p>
          </div>
        ) : (
          <div className="placeholder">
            <h1>ade</h1>
            <p className="muted">
              Add a project with the + button to get started.
            </p>
          </div>
        )}
      </section>
    </main>
  );
}

export default App;
