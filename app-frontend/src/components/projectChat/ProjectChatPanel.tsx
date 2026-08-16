// PM chat panel (E17-04, #58): the project view's right panel, hosting the
// one persistent project-scoped ACP conversation. This wires the
// conversation lifecycle (get-or-create → session start → transcript); the
// PM system prompt and the proposal-block approval card build on top.
// Restyled to design_handoff_v2 §5c: 46px header ("PM" + mono scope/chord),
// with the transcript/composer voice scoped under .pm-chat.

import { useEffect, useState } from "react";
import { acpStart, getAppSetting, listProviders } from "../../lib/tauri";
import type { ProviderDto } from "../../lib/tauri";
import { hint } from "../../lib/useCommands";
import { useConversations } from "../../store/conversations";
import { useUi } from "../../store/ui";
import ConversationView from "../ConversationView";

export default function ProjectChatPanel({ projectId }: { projectId: string }) {
  const ownerKey = `project:${projectId}`;
  const [conversationId, setConversationId] = useState<string | null>(null);
  const [provider, setProvider] = useState<ProviderDto | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Re-render on keymap edits so the header chord stays live (E14).
  useUi((s) => s.bindingsVersion);
  const chord = hint("toggle-project-chat") || "⌘⇧2";

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        // #126: honor the defaultAgent app setting when it names an
        // ACP-capable provider; otherwise fall back to the first ACP entry
        // in the registry (the PM chat has no TUI path).
        const acp = (await listProviders()).filter((p) =>
          p.capabilities.includes("acp"),
        );
        if (acp.length === 0) throw new Error("no ACP-capable provider available");
        const wanted = (await getAppSetting("defaultAgent").catch(() => null)) as
          | string
          | null;
        const provider = acp.find((p) => p.id === wanted) ?? acp[0];
        if (!cancelled) setProvider(provider);
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
    <aside className="project-chat pm-chat">
      <header className="project-chat-header">
        <span>PM</span>
        <span className="fc-changes-header-side">
          <span className="pm-chat-scope">
            {provider
              ? `${provider.name}${provider.defaultModel ? ` · ${provider.defaultModel}` : ""} · `
              : ""}
            {`project root · ${chord}`}
          </span>
          <button
            className="fc-sheet-close"
            aria-label="Close panel"
            title="Close panel"
            onClick={() => {
              const ui = useUi.getState();
              ui.setProjectChatOpen(false);
              ui.setChangesOpen(false); // close the sheet, not just switch modes
            }}
          >
            ×
          </button>
        </span>
      </header>
      {error && <p className="error">{error}</p>}
      {conversationId ? (
        <ConversationView conversationId={conversationId} ownerKey={ownerKey} active />
      ) : (
        !error && (
          <div className="chat-hero">
            <p className="muted">Starting the project agent…</p>
          </div>
        )
      )}
    </aside>
  );
}
