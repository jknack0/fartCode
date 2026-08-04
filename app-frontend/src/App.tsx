// ade app shell (E1-04/E2-08/E2-10): sidebar + task view with tabs.

import { useEffect } from "react";
import CommandPalette from "./components/CommandPalette";
import Onboarding from "./components/Onboarding";
import ResourceMonitor from "./components/ResourceMonitor";
import Sidebar from "./components/Sidebar";
import TaskView from "./components/TaskView";
import { useTaskShortcuts } from "./lib/shortcuts";
import { useSidebar, wireSidebarEvents } from "./store/sidebar";
import { wireTabsEvents } from "./store/tabs";
import { useUi } from "./store/ui";

function App() {
  const load = useSidebar((s) => s.load);
  const selectedProjectId = useSidebar((s) => s.selectedProjectId);
  const selectedTaskId = useSidebar((s) => s.selectedTaskId);
  const error = useSidebar((s) => s.error);
  const setPaletteOpen = useUi((s) => s.setPaletteOpen);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPaletteOpen(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [setPaletteOpen]);

  useEffect(() => {
    load().catch(() => {});
    const unlisten = wireSidebarEvents();
    const unlistenTabs = wireTabsEvents();
    return () => {
      unlisten();
      unlistenTabs();
    };
  }, [load]);

  useTaskShortcuts();

  return (
    <main className="shell">
      <Sidebar />
      <Onboarding />
      <CommandPalette />
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
