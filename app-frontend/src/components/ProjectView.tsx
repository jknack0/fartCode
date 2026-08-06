// Project view (E17, ARCHITECTURE.md §13): the planning surface that
// replaces the old "coming in Phase 1" stub — issue board (primary) + PM
// chat (collapsible right panel, ⌘⇧2). The two surfaces own their own
// components and stylesheets; this shell owns the layout and the header
// actions (GitHub remote link, chat toggle).

import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-shell";
import BoardView from "./board/BoardView";
import CardDetail from "./board/CardDetail";
import ProjectChatPanel from "./projectChat/ProjectChatPanel";
import { IconChat, IconGitHub } from "./icons";
import { projectGithubUrl } from "../lib/tauri";
import { hint } from "../lib/useCommands";
import { useSidebar } from "../store/sidebar";
import { useUi } from "../store/ui";

export default function ProjectView({ projectId }: { projectId: string }) {
  const chatOpen = useUi((s) => s.projectChatOpen);
  const detailIssueId = useUi((s) => s.boardDetailIssueId);
  const projectName = useSidebar(
    (s) => s.projects.find((p) => p.id === projectId)?.name ?? null,
  );
  const [githubUrl, setGithubUrl] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    projectGithubUrl(projectId)
      .then((url) => !cancelled && setGithubUrl(url))
      .catch(() => !cancelled && setGithubUrl(null));
    return () => {
      cancelled = true;
    };
  }, [projectId]);

  // Card detail takes precedence over the chat panel in the right region;
  // closing detail (or the chat toggle) falls back to the PM chat.
  const showDetail = detailIssueId !== null;
  return (
    <div className="project-view">
      <div className="project-main">
        <header className="project-header">
          <span className="project-title">{projectName ?? "Project"}</span>
          <span className="project-actions">
            {githubUrl && (
              <button
                className="project-action"
                title="Open on GitHub"
                onClick={() => void open(githubUrl)}
              >
                <IconGitHub />
              </button>
            )}
            <button
              className={`project-action${chatOpen ? " active" : ""}`}
              title={`Project chat (${hint("toggle-project-chat") || "⌘⇧2"})`}
              onClick={() => useUi.getState().setProjectChatOpen(!chatOpen)}
            >
              <IconChat />
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
