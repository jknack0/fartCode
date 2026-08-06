// Project view (E17, ARCHITECTURE.md §13): the planning surface that
// replaces the old "coming in Phase 1" stub — issue board (primary) + PM
// chat (collapsible right panel, ⌘⇧2). The two surfaces own their own
// components and stylesheets; this shell owns the layout and the header
// actions (GitHub remote link, chat toggle).

import BoardView from "./board/BoardView";
import CardDetail from "./board/CardDetail";
import ProjectChatPanel from "./projectChat/ProjectChatPanel";
import { IconChat, IconGitHub } from "./icons";
import { hint } from "../lib/useCommands";
import { useSidebar } from "../store/sidebar";
import { useUi } from "../store/ui";

export default function ProjectView({ projectId }: { projectId: string }) {
  const chatOpen = useUi((s) => s.projectChatOpen);
  const detailIssueId = useUi((s) => s.boardDetailIssueId);
  const projectName = useSidebar(
    (s) => s.projects.find((p) => p.id === projectId)?.name ?? null,
  );

  // Card detail takes precedence over the chat panel in the right region;
  // closing detail (or the chat toggle) falls back to the PM chat.
  const showDetail = detailIssueId !== null;
  return (
    <div className="project-view">
      <div className="project-main">
        <header className="project-header">
          <span className="project-title">{projectName ?? "Project"}</span>
          <span className="project-actions">
            <button
              className="project-action"
              title={`Project changes (${hint("toggle-changes") || "⌘⇧1"})`}
              onClick={() => useUi.getState().setChangesOpen(true)}
            >
              <IconGitHub size={14} />
            </button>
            <button
              className={`project-action${chatOpen ? " active" : ""}`}
              title={`Project chat (${hint("toggle-project-chat") || "⌘⇧2"})`}
              onClick={() => useUi.getState().setProjectChatOpen(!chatOpen)}
            >
              <IconChat size={14} />
            </button>
          </span>
        </header>
        <BoardView projectId={projectId} />
      </div>
      {(showDetail || chatOpen) && (
        <div className="project-side">
          {showDetail ? (
            <CardDetail projectId={projectId} issueId={detailIssueId} />
          ) : (
            <ProjectChatPanel projectId={projectId} />
          )}
        </div>
      )}
    </div>
  );
}
