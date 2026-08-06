// Project view (E17, ARCHITECTURE.md §13): the issue board. The header
// lives at App level (full-width row, ProjectHeader); the right sheet
// (ChangesSidebar) alternates between changes and the PM chat.
import BoardView from "./board/BoardView";

export default function ProjectView({ projectId }: { projectId: string }) {
  return (
    <div className="project-view">
      <div className="project-main">
        <BoardView projectId={projectId} />
      </div>
    </div>
  );
}
