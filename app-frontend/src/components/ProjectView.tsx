// Project view (E17, ARCHITECTURE.md §13): the planning surface — issue
// board (primary) + header actions. The right-side sheet (App-level
// ChangesSidebar) hosts git changes on top with the PM chat docked at the
// bottom; a card click swaps the sheet to card detail.

import BoardView from "./board/BoardView";
import { IconChat, IconGitHub } from "./icons";
import { hint } from "../lib/useCommands";
import { useSidebar } from "../store/sidebar";
import { useUi } from "../store/ui";

export default function ProjectView({ projectId }: { projectId: string }) {
  const chatOpen = useUi((s) => s.projectChatOpen);
  const changesOpen = useUi((s) => s.changesOpen);
  const projectName = useSidebar(
    (s) => s.projects.find((p) => p.id === projectId)?.name ?? null,
  );

  return (
    <div className="project-view">
      <div className="project-main">
        <header className="project-header">
          <span className="project-title">{projectName ?? "Project"}</span>
          <span className="project-actions">
            <button
              className={`project-action${changesOpen ? " active" : ""}`}
              title={`Project changes (${hint("toggle-changes") || "⌘⇧1"})`}
              onClick={() => useUi.getState().setChangesOpen(!changesOpen)}
            >
              <IconGitHub size={14} />
            </button>
            <button
              className={`project-action${chatOpen && changesOpen ? " active" : ""}`}
              title={`Project chat (${hint("toggle-project-chat") || "⌘⇧2"})`}
              onClick={() => {
                const ui = useUi.getState();
                const next = !ui.projectChatOpen;
                ui.setProjectChatOpen(next);
                if (next) ui.setChangesOpen(true);
              }}
            >
              <IconChat size={14} />
            </button>
          </span>
        </header>
        <BoardView projectId={projectId} />
      </div>
    </div>
  );
}
