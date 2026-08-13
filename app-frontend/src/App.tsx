// fartCode app shell (E1-04/E2-08/E2-10/E14-01): sidebar + task view with tabs.
// All keyboard shortcuts are registry commands (lib/commands.ts); the single
// dispatch listener is installed by useCommands().

import { useEffect } from "react";
import ChangesSidebar from "./components/ChangesSidebar";
import CommandPalette from "./components/CommandPalette";
import DossierConsentCard from "./components/DossierConsentCard";
import Modals from "./components/Modals";
import Warnings from "./components/Warnings";
import Onboarding from "./components/Onboarding";
import ProjectView from "./components/ProjectView";
import ResourceMonitor from "./components/ResourceMonitor";
import Nav from "./components/Nav";
import TaskView from "./components/TaskView";
import { useCommands, hint } from "./lib/useCommands";
import { wireChangesEvents } from "./store/changes";
import { useDossierConsent } from "./store/dossierConsent";
import { wireDiffsEvents } from "./store/diffs";
import { wireOpenFileIntents } from "./lib/editor-tabs";
import { useSidebar, wireSidebarEvents } from "./store/sidebar";
import { useConversations, wireConversationEvents } from "./store/conversations";
import { wireTabsEvents } from "./store/tabs";
import { wireLineCommentEvents } from "./store/line-comments";
import { wireDependencyEvents } from "./store/dependencies";
import { wirePrEvents } from "./store/pr";
import { wireStepEvents } from "./store/steps";
import { loadNotificationSetting, wireNotificationEvents } from "./lib/notifications";

function App() {
  const load = useSidebar((s) => s.load);
  const selectedProjectId = useSidebar((s) => s.selectedProjectId);
  const selectedTaskId = useSidebar((s) => s.selectedTaskId);
  const error = useSidebar((s) => s.error);

  useEffect(() => {
    load().catch(() => {});
    const unlisten = wireSidebarEvents();
    const unlistenTabs = wireTabsEvents();
    const unwireOpenFile = wireOpenFileIntents();
    const unlistenConversations = wireConversationEvents();
    const unlistenChanges = wireChangesEvents();
    const unlistenDiffs = wireDiffsEvents();
    const unlistenComments = wireLineCommentEvents();
    const unlistenPr = wirePrEvents();
    const unlistenDependencies = wireDependencyEvents();
    // App-lifetime on purpose: `step:launch` is a directive, and the act of
    // carrying it out navigates to the task view. A listener owned by the
    // board would unmount itself the moment it fired, losing every
    // settle-chained launch (and the step-done/queued flags with it).
    const unlistenSteps = wireStepEvents();
    const unlistenNotifications = wireNotificationEvents();
    void loadNotificationSetting();
    return () => {
      unlisten();
      unlistenTabs();
      unwireOpenFile();
      unlistenConversations();
      unlistenChanges();
      unlistenDiffs();
      unlistenComments();
      unlistenPr();
      unlistenDependencies();
      unlistenSteps();
      unlistenNotifications();
    };
  }, [load]);
  const ensureConversations = useConversations((s) => s.ensure);
  useEffect(() => {
    if (selectedTaskId) void ensureConversations(selectedTaskId).catch(() => {});
  }, [selectedTaskId, ensureConversations]);

  // #74: a consent card asking about a project the user has navigated away
  // from is withdrawn. Consent belongs to the project that was ASKED —
  // answering it here, after a switch, would write the answer to a project
  // nobody was asked about (and could flip an explicit decline back on).
  // App owns this because App owns the selection; the board is not
  // remounted by a project switch and cannot see it.
  useEffect(() => {
    useDossierConsent.getState().cancelForeignAsk(selectedProjectId);
  }, [selectedProjectId]);

  useCommands();

  return (
    <main className="shell">
      <Nav />
      <Onboarding />
      <CommandPalette />
      {/* App-level, beside Onboarding: two of the three surfaces that can
          start an agent step live in the task view, where the board is
          unmounted (#74 §8e). */}
      <DossierConsentCard />
      <Modals />
      <Warnings />
      <ResourceMonitor />
      <section className="main">
        {error && <p className="error">{error}</p>}
        {selectedTaskId && selectedProjectId ? (
          <TaskView taskId={selectedTaskId} />
        ) : selectedProjectId ? (
          <ProjectView projectId={selectedProjectId} />
        ) : (
          <div className="placeholder">
            <h1 className="brand">
              fart<span className="brand-accent">Code</span>
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
