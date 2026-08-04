// Conversation list sidebar (E2-08): ⌘T create, ⌘D right-split, click to
// activate, close button, active highlight.

import { useEffect } from "react";
import { useConversations } from "../store/conversations";

export default function ConversationList({
  taskId,
  projectId,
}: {
  taskId: string;
  projectId: string;
}) {
  const { conversations, activeId, load, create, remove, setActive } =
    useConversations();

  useEffect(() => {
    load(taskId);
  }, [taskId]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "t" && !e.shiftKey) {
        e.preventDefault();
        create(taskId, projectId);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [taskId, projectId]);

  return (
    <div className="conversation-list">
      <div className="conversation-list-header">
        <span>Conversations</span>
        <button
          title="New conversation (⌘T)"
          onClick={() => create(taskId, projectId)}
        >
          +
        </button>
      </div>
      <ul>
        {conversations.map((c) => (
          <li
            key={c.id}
            className={c.id === activeId ? "active" : ""}
            onClick={() => setActive(c.id)}
          >
            <span className="conv-title">{c.title}</span>
            {c.provider && <span className="badge">{c.provider}</span>}
            {c.agentStatus && (
              <span className={`status ${c.agentStatus.toLowerCase()}`}>
                {c.agentStatus}
              </span>
            )}
            <button
              className="close-btn"
              title="Close conversation"
              onClick={(e) => {
                e.stopPropagation();
                remove(projectId, taskId, c.id);
              }}
            >
              ×
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}
