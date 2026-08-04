// Conversation view (E2-08): terminal placeholder, message input,
// context pills. Conversations live under tasks — no separate list panel.
// Input shortcuts are commands (E14-01): ⌘Enter send, ⌘⇧A add context.

import { useEffect } from "react";
import { useConversations } from "../store/conversations";

export default function ConversationView({
  taskId,
}: {
  taskId: string;
}) {
  const draftPrompt = useConversations((s) => s.draftPrompt);
  const contextItems = useConversations((s) => s.contextItems);
  const setDraftPrompt = useConversations((s) => s.setDraftPrompt);
  const removeContextItem = useConversations((s) => s.removeContextItem);
  const send = useConversations((s) => s.send);
  const load = useConversations((s) => s.load);

  useEffect(() => { load(taskId); }, [taskId]);

  return (
    <div className="conversation-view">
      <div className="terminal-pane">
        <div className="terminal-placeholder">
          <span className="muted">Terminal — starts when the agent launches</span>
        </div>
      </div>

      {contextItems.length > 0 && (
        <div className="context-pills">
          {contextItems.map((item, i) => (
            <span key={i} className="context-pill">
              {item.kind === "file" ? <>{item.path}</> : <>{item.text.slice(0, 60)}</>}
              <button onClick={() => removeContextItem(i)} title="Remove">&times;</button>
            </span>
          ))}
        </div>
      )}

      <div className="message-input-bar">
        <textarea
          value={draftPrompt}
          onChange={(e) => setDraftPrompt(e.target.value)}
          placeholder="Type a message"
          rows={2}
        />
        <button className="primary" onClick={send} disabled={!draftPrompt.trim()} title="⌘Enter">
          Send
        </button>
      </div>
    </div>
  );
}
