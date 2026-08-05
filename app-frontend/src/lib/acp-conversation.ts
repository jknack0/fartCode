// ACP conversation helpers: find-or-create the task's ACP conversation
// (the runtime type is decided SERVER-SIDE from the provider decision —
// E2-11-5; the renderer never picks it) and focus its tab. Shared by the
// ⌘⇧A command and the diff selection popover (E4 selection → agent).
import { listProviders, type ConversationDto } from "./tauri";
import { useConversations } from "../store/conversations";
import { useTabs, type PaneId } from "../store/tabs";

/** Returns the task's ACP conversation, creating one with the first
 * ACP-capable provider when none exists. `null` when no ACP provider is
 * available or the provider resolves to the TUI path. */
export async function ensureAcpConversation(
  projectId: string,
  taskId: string,
): Promise<ConversationDto | null> {
  await useConversations.getState().ensure(taskId);
  let conv = useConversations.getState().activeAcp(taskId);
  if (conv) return conv;

  const providers = await listProviders();
  const acp = providers.find((p) => p.capabilities.includes("acp"));
  if (!acp) {
    // Unreachable with the compiled-in registry (claude is ACP-capable);
    // guarded so a future registry change degrades loudly, not weirdly.
    console.warn("ensure-acp: no ACP-capable provider available");
    return null;
  }
  conv = await useConversations.getState().create(projectId, taskId, acp.id, "Agent");
  if (conv.runtime !== "acp") {
    console.warn("ensure-acp: provider resolved to the TUI path", conv.provider);
    return null;
  }
  return conv;
}

/** Focuses the conversation's tab wherever it lives, opening it in `pane`
 * when it has no tab yet. */
export function focusConversationTab(
  taskId: string,
  conversationId: string,
  pane: PaneId,
): void {
  const tabs = useTabs.getState();
  const panes = tabs.panesByTask[taskId];
  for (const p of ["left", "right"] as const) {
    if (panes?.[p]?.tabs.some((t) => t.id === conversationId)) {
      tabs.setActiveTab(taskId, p, conversationId);
      return;
    }
  }
  tabs.addTab(taskId, pane, {
    id: conversationId,
    kind: "conversation",
    title: "Agent",
  });
}
