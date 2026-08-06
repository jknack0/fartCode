// Project view (E17, ARCHITECTURE.md §13): the planning surface that
// replaces the old "coming in Phase 1" stub — issue board (primary) + PM
// chat (collapsible right panel, ⌘⇧2). The two surfaces own their own
// components and stylesheets; this shell owns only the layout.

import BoardView from "./board/BoardView";
import CardDetail from "./board/CardDetail";
import ProjectChatPanel from "./projectChat/ProjectChatPanel";
import { useUi } from "../store/ui";

export default function ProjectView({ projectId }: { projectId: string }) {
  const chatOpen = useUi((s) => s.projectChatOpen);
  const detailIssueId = useUi((s) => s.boardDetailIssueId);
  // Card detail takes precedence over the chat panel in the right region;
  // closing detail (or the chat toggle) falls back to the PM chat.
  const showDetail = detailIssueId !== null;
  return (
    <div className="project-view">
      <BoardView projectId={projectId} />
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
