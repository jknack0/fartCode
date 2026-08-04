// ade app shell (E1-04/E2-08): sidebar + main view.

import { useEffect } from "react";
import CommandPalette from "./components/CommandPalette";
import ConversationView from "./components/ConversationView";
import Onboarding from "./components/Onboarding";
import ResourceMonitor from "./components/ResourceMonitor";
import Sidebar from "./components/Sidebar";
import { useSidebar, wireSidebarEvents } from "./store/sidebar";
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
    return () => unlisten();
  }, [load]);

  return (
    <main className="shell">
      <Sidebar />
      <Onboarding />
      <CommandPalette />
      <ResourceMonitor />
      <section className="main">
        {error && <p className="error">{error}</p>}
        {selectedTaskId && selectedProjectId ? (
          <ConversationView taskId={selectedTaskId} />
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
