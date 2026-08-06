// Project header (E17 dogfood): spans the shell's full width above the
// board AND the changes/chat sheet, so the right sheet never covers it.
// Hosts the sheet controls: changes (git) and PM chat alternate inside the
// one right sheet — opening one replaces the other, never both at once.

import { IconChat, IconGitHub } from "./icons";
import { hint } from "../lib/useCommands";
import { useSidebar } from "../store/sidebar";
import { useUi } from "../store/ui";

export default function ProjectHeader({ projectId }: { projectId: string }) {
  const chatOpen = useUi((s) => s.projectChatOpen);
  const changesOpen = useUi((s) => s.changesOpen);
  const projectName = useSidebar(
    (s) => s.projects.find((p) => p.id === projectId)?.name ?? null,
  );

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
