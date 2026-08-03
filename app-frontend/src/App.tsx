// ade app shell (E1-04): sidebar + main view. The project view is a Phase 0
// stub ("Project chat — coming in Phase 1"); task tabs arrive with their
// tickets.
import { useEffect } from "react";
import CommandPalette from "./components/CommandPalette";
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

  // ⌘K / Ctrl+K → command palette.
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
        {selectedTaskId ? (
          <div className="placeholder">
            <h1>Task {selectedTaskId.slice(0, 8)}</h1>
            <p className="muted">Task tabs (chat, terminal, files) arrive with E2-series tickets.</p>
          </div>
        ) : selectedProjectId ? (
          <div className="placeholder">
            <h1>Project chat</h1>
            <p className="muted">Coming in Phase 1 — project-level agent chat.</p>
          </div>
        ) : (
          <div className="placeholder">
            <h1>ade</h1>
            <p className="muted">
              Add a project with ⌘⇧N to get started. The agent experience lands
              with Phase 0 tickets.
            </p>
          </div>
        )}
      </section>
    </main>
  );
}

export default App;
