// Task chat panel: the task view's right panel, hosting the task's agent
// conversation in the sheet — the same surface the PM chat uses at project
// scope. The conversation lifecycle mirrors the tab chat (⌘⇧A): get-or-
// create → session start → transcript. Rendered by ChangesSidebar when the
// task's sheet is in chat mode.

import { useEffect, useState } from "react";
import { acpStart } from "../lib/tauri";
import { ensureAcpConversation } from "../lib/acp-conversation";
import { useUi } from "../store/ui";
import { IconChevron } from "./icons";
import ConversationView from "./ConversationView";

export default function TaskChatPanel({
  projectId,
  taskId,
}: {
  projectId: string;
  taskId: string;
}) {
  const [conversationId, setConversationId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const conv = await ensureAcpConversation(projectId, taskId);
        if (cancelled) return;
        if (!conv) {
          setError("No ACP-capable agent available for this task.");
          return;
        }
        await acpStart(conv.id);
        if (!cancelled) setConversationId(conv.id);
      } catch (e) {
        if (!cancelled) setError(String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [projectId, taskId]);

  return (
    <aside className="project-chat">
      <header className="project-chat-header">
        <span>Task chat</span>
        <button
          className="project-chat-minimize"
          title="Hide task chat (⌘⇧A)"
          aria-label="Hide task chat"
          onClick={() => {
            const ui = useUi.getState();
            ui.setTaskChatOpen(false);
            ui.setChangesOpen(false); // close the sheet, not just switch modes
          }}
        >
          <IconChevron />
        </button>
      </header>
      {error && <p className="error">{error}</p>}
      {conversationId ? (
        <ConversationView conversationId={conversationId} ownerKey={taskId} active />
      ) : (
        !error && <p className="muted">Starting the task agent…</p>
      )}
    </aside>
  );
}
