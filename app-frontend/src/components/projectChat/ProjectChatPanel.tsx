// PM chat panel (E17-04, #58): the project view's right panel, hosting the
// one persistent project-scoped ACP conversation. This wires the
// conversation lifecycle (get-or-create → session start → transcript); the
// PM system prompt and the proposal-block approval card build on top.

import { useEffect, useState } from "react";
import { acpStart, listProviders } from "../../lib/tauri";
import { useConversations } from "../../store/conversations";
import ConversationView from "../ConversationView";

export default function ProjectChatPanel({ projectId }: { projectId: string }) {
  const ownerKey = `project:${projectId}`;
  const [conversationId, setConversationId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const provider = (await listProviders()).find((p) =>
          p.capabilities.includes("acp"),
        );
        if (!provider) throw new Error("no ACP-capable provider available");
        const conv = await useConversations
          .getState()
          .ensureProject(projectId, provider.id);
        if (cancelled) return;
        await acpStart(conv.id);
        if (!cancelled) setConversationId(conv.id);
      } catch (e) {
        if (!cancelled) setError(String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [projectId]);

  return (
    <aside className="project-chat">
      <header className="project-chat-header">Project chat</header>
      {error && <p className="error">{error}</p>}
      {conversationId ? (
        <ConversationView conversationId={conversationId} ownerKey={ownerKey} active />
      ) : (
        !error && <p className="muted">Starting the project agent…</p>
      )}
    </aside>
  );
}
