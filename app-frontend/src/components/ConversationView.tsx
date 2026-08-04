// Conversation view (E2-08): terminal placeholder, message input,
// context pills, ⌘Enter send, ⌘⇧A add context.

import { useEffect, useRef } from "react";
import { useConversations, type ContextItem } from "../store/conversations";

export default function ConversationView(_props: {
  projectId: string;
  taskId: string;
}) {
  const conversations = useConversations((s) => s.conversations);
  const activeId = useConversations((s) => s.activeId);
  const draftPrompt = useConversations((s) => s.draftPrompt);
  const contextItems = useConversations((s) => s.contextItems);
  const setDraftPrompt = useConversations((s) => s.setDraftPrompt);
  const addContextFile = useConversations((s) => s.addContextFile);
  const addContextPrompt = useConversations((s) => s.addContextPrompt);
  const removeContextItem = useConversations((s) => s.removeContextItem);
  const clearContext = useConversations((s) => s.clearContext);

  const inputRef = useRef<HTMLTextAreaElement>(null);
  const active = conversations.find((c) => c.id === activeId) ?? null;

  const send = () => {
    if (!draftPrompt.trim()) return;
    // E2-08 Phase 0: if no active conversation, create one then send.
    // The prompt is recorded on the next E2-06 agent launch; for now the
    // input clears and the context pills reset.
    clearContext();
    inputRef.current?.focus();
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const meta = e.metaKey || e.ctrlKey;
      if (meta && e.key === "Enter") {
        e.preventDefault();
        send();
      }
      if (meta && e.shiftKey && e.key === "a") {
        e.preventDefault();
        // Show a tiny inline prompt for file path or prompt text.
        const raw = prompt("Add context — path to a file, or '@ text':");
        if (!raw) return;
        if (raw.startsWith("@")) {
          addContextPrompt(raw.slice(1).trim());
        } else {
          addContextFile(raw.trim());
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [draftPrompt, contextItems]);

  return (
    <div className="conversation-view">
      <div className="conversation-header">
        <h2>{active?.title ?? "Select a conversation"}</h2>
        {active?.provider && (
          <span className="badge provider">{active.provider}</span>
        )}
      </div>

      {/* Terminal placeholder — xterm.js attaches here (PTY-event plumbing follow-up) */}
      <div className="terminal-pane">
        <div className="terminal-placeholder">
          <span className="muted">Terminal — starts when the agent launches</span>
        </div>
      </div>

      {/* Context pills */}
      {contextItems.length > 0 && (
        <div className="context-pills">
          {contextItems.map((item, i) => (
            <Pill key={i} item={item} onRemove={() => removeContextItem(i)} />
          ))}
        </div>
      )}

      {/* Message input */}
      <div className="message-input-bar">
        <textarea
          ref={inputRef}
          value={draftPrompt}
          onChange={(e) => setDraftPrompt(e.target.value)}
          onKeyDown={(e) => {
            if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
              e.preventDefault();
              send();
            }
          }}
          placeholder="Type a message… (⌘Enter to send, ⌘⇧A for context)"
          rows={2}
        />
        <button className="primary" onClick={send} disabled={!draftPrompt.trim()}>
          Send
        </button>
      </div>
    </div>
  );
}

function Pill({ item, onRemove }: { item: ContextItem; onRemove: () => void }) {
  return (
    <span className="context-pill">
      {item.kind === "file" ? (
        <>📄 {item.path}</>
      ) : (
        <>💬 {item.text.slice(0, 60)}</>
      )}
      <button onClick={onRemove} title="Remove">×</button>
    </span>
  );
}
