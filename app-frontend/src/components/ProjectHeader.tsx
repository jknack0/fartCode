// Project header (E17 dogfood): spans the shell's full width above the
// board AND the changes/chat sheet. Hosts the sheet controls (changes ⇄
// chat alternate in the one right sheet) and "Sync from GitHub" — looks up
// the project's open GitHub issues and creates a native board card for
// each one not already imported (deduped by URL).

import { useState } from "react";
import { projectGithubIssues, issueImportGithub, issueList } from "../lib/tauri";
import { IconBranch, IconChat, IconGitHub } from "./icons";
import { hint } from "../lib/useCommands";
import { useSidebar } from "../store/sidebar";
import { useUi } from "../store/ui";

export default function ProjectHeader({ projectId }: { projectId: string }) {
  const chatOpen = useUi((s) => s.projectChatOpen);
  const changesOpen = useUi((s) => s.changesOpen);
  const projectName = useSidebar(
    (s) => s.projects.find((p) => p.id === projectId)?.name ?? null,
  );
  const [syncing, setSyncing] = useState(false);
  const [syncNote, setSyncNote] = useState<string | null>(null);

  const syncGithub = async () => {
    setSyncing(true);
    setSyncNote(null);
    try {
      const [ghIssues, board] = await Promise.all([
        projectGithubIssues(projectId),
        issueList(projectId),
      ]);
      const imported = new Set(
        board.filter((i) => i.externalRef).map((i) => i.externalRef),
      );
      const fresh = ghIssues.filter((g) => !imported.has(g.url));
      for (const g of fresh) {
        await issueImportGithub({
          projectId,
          number: g.number,
          title: g.title,
          url: g.url,
          labels: g.labels,
        });
      }
      setSyncNote(
        fresh.length > 0
          ? `Imported ${fresh.length} GitHub ${fresh.length === 1 ? "issue" : "issues"}`
          : "Already synced",
      );
    } catch (e) {
      setSyncNote(String(e));
    } finally {
      setSyncing(false);
      setTimeout(() => setSyncNote(null), 4000);
    }
  };

  return (
    <header className="app-header">
      <span className="app-header-title">{projectName ?? "Project"}</span>
      <span className="app-header-actions">
        <button
          className={`project-action${changesOpen && !chatOpen ? " active" : ""}`}
          title={`Project changes (${hint("toggle-changes") || "⌘⇧1"})`}
          onClick={() => {
            const ui = useUi.getState();
            if (!ui.changesOpen) {
              ui.setChangesOpen(true);
              ui.setProjectChatOpen(false);
            } else if (ui.projectChatOpen) {
              ui.setProjectChatOpen(false); // chat mode → changes mode
            } else {
              ui.setChangesOpen(false); // changes mode → close the sheet
            }
          }}
        >
          <IconBranch size={14} />
        </button>
        {syncNote && <span className="project-sync-note">{syncNote}</span>}
        <button
          className="project-action"
          title="Sync open GitHub issues onto the board"
          disabled={syncing}
          onClick={() => void syncGithub()}
        >
          <IconGitHub size={14} />
        </button>
        <button
          className={`project-action${chatOpen && changesOpen ? " active" : ""}`}
          title={`Project chat (${hint("toggle-project-chat") || "⌘⇧2"})`}
          onClick={() => {
            const ui = useUi.getState();
            if (!ui.changesOpen) {
              ui.setProjectChatOpen(true);
              ui.setChangesOpen(true);
            } else if (ui.projectChatOpen) {
              ui.setChangesOpen(false); // chat mode → close the sheet
            } else {
              ui.setProjectChatOpen(true); // changes mode → chat mode
            }
          }}
        >
          <IconChat size={14} />
        </button>
      </span>
    </header>
  );
}
