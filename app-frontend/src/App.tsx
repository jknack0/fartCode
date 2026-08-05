// ade app shell (E1-04/E2-08/E2-10/E14-01): sidebar + task view with tabs.
// All keyboard shortcuts are registry commands (lib/commands.ts); the single
// dispatch listener is installed by useCommands().

import { useEffect } from "react";
import ChangesSidebar from "./components/ChangesSidebar";
import CommandPalette from "./components/CommandPalette";
import Modals from "./components/Modals";
import Onboarding from "./components/Onboarding";
import ResourceMonitor from "./components/ResourceMonitor";
import Sidebar from "./components/Sidebar";
import TaskView from "./components/TaskView";
import { useCommands, hint } from "./lib/useCommands";
import { wireChangesEvents } from "./store/changes";
import { wireDiffsEvents } from "./store/diffs";
import { useSidebar, wireSidebarEvents } from "./store/sidebar";
import { useConversations, wireConversationEvents } from "./store/conversations";
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
    const unlistenConversations = wireConversationEvents();
    const unlistenChanges = wireChangesEvents();
    const unlistenDiffs = wireDiffsEvents();
    return () => {
      unlisten();
      unlistenTabs();
      unlistenConversations();
      unlistenChanges();
      unlistenDiffs();
    };
  }, [load]);
  const ensureConversations = useConversations((s) => s.ensure);
  useEffect(() => {
    if (selectedTaskId) void ensureConversations(selectedTaskId).catch(() => {});
  }, [selectedTaskId, ensureConversations]);

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
            <h1>
              a<span className="brand-mark" aria-hidden="true" />de
            </h1>
            <p className="muted">
              Add a project to get started — press{" "}
              <span className="kbd-hint">{hint("new-project") || "⌘⇧N"}</span>{" "}
              or the + button.
            </p>
          </div>
        )}
      </section>
      <ChangesSidebar />
    </main>
  );
}

export default App;
