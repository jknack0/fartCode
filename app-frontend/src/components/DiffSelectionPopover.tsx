// Diff selection popover: highlight text in a diff editor and a floating
// "Ask agent" button appears at the selection; click it (or it stays for
// the current selection) and a prompt box opens — Enter submits the
// selection (file, lines, code) plus the typed prompt to the task's ACP
// conversation via the shared send path, then focuses the conversation tab
// so the result is visible. Escape closes without sending.
import { useEffect, useRef, useState, type RefObject } from "react";
import { MergeView } from "@codemirror/merge";
import { ensureAcpConversation, focusConversationTab, focusOrOpenTab } from "../lib/acp-conversation";
import { getDiffView } from "../lib/diff-views";
import { terminalListForTask, terminalWrite } from "../lib/tauri";
import { useConversations } from "../store/conversations";
import { useDiffs, type DiffParams, type DiffSelection } from "../store/diffs";
import { useSidebar } from "../store/sidebar";
import { useTabs, type PaneId } from "../store/tabs";

export default function DiffSelectionPopover({
  tabId,
  params,
  taskId,
  containerRef,
}: {
  tabId: string;
  params: DiffParams;
  taskId: string;
  containerRef: RefObject<HTMLDivElement | null>;
}) {
  const selection = useDiffs((s) => s.selectionByTab[tabId] ?? null);
  const projectId = useSidebar((s) => s.selectedProjectId);
  const [open, setOpen] = useState(false);
  const [prompt, setPrompt] = useState("");
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [destination, setDestination] = useState<string | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);

  // A collapsed/changed selection resets the popover around the new one.
  useEffect(() => {
    if (!selection) {
      setOpen(false);
      setPrompt("");
      setError(null);
    }
  }, [selection]);

  useEffect(() => {
    if (open) textareaRef.current?.focus();
  }, [open]);

  // Where will the prompt go? Resolved when the popover opens so the user
  // sees the routing (agent terminal vs ACP chat) before sending.
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    terminalListForTask(taskId)
      .then((terms) => {
        if (cancelled) return;
        const agent = terms.find((t) => t.agent !== null);
        setDestination(agent ? `${agent.agent} terminal` : "Agent chat");
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [open, taskId]);

  if (!selection || !projectId) return null;

  const position = computePosition(tabId, selection, containerRef.current);
  if (!position) return null;

  const close = () => {
    setOpen(false);
    setPrompt("");
    setError(null);
    useDiffs.getState().setSelection(tabId, null);
  };

  const submit = async () => {
    const text = prompt.trim();
    if (!text || sending) return;
    setSending(true);
    setError(null);
    try {
      const side = selection.side === "a" ? " (baseline)" : "";
      const full =
        `${params.path} lines ${selection.fromLine}–${selection.toLine}${side}:\n` +
        "```\n" +
        selection.text +
        "\n```\n\n" +
        text;

      // Route: the task's live agent terminal first — the work context
      // lives there, and the prompt lands where the user is already
      // watching. ACP chat is the fallback when no agent terminal exists.
      const terminals = await terminalListForTask(taskId);
      const agentTerm = terminals.find((t) => t.agent !== null);
      const tabs = useTabs.getState();
      const pane: PaneId =
        tabs.panesByTask[taskId]?.right && tabs.activePaneByTask[taskId] === "right"
          ? "right"
          : "left";

      if (agentTerm) {
        // Bracketed paste so multi-line content lands as one block, then
        // Enter to submit.
        await terminalWrite(agentTerm.id, `[200~${full}[201~\r`);
        useDiffs.getState().setSelection(tabId, null);
        focusOrOpenTab(taskId, agentTerm.id, "terminal", agentTerm.agent ?? "Agent", pane);
        return;
      }

      const conv = await ensureAcpConversation(projectId, taskId);
      if (!conv) throw new Error("no agent available for this task (no terminal, no ACP provider)");
      await useConversations.getState().sendPrompt(conv.id, full);
      useDiffs.getState().setSelection(tabId, null);
      focusConversationTab(taskId, conv.id, pane);
    } catch (e) {
      setError(String(e));
      setSending(false);
    }
  };

  if (!open) {
    return (
      <button
        className="diff-sel-fab"
        style={{ left: position.left, top: position.top }}
        onClick={() => setOpen(true)}
      >
        Ask agent
      </button>
    );
  }

  return (
    <div
      className="diff-sel-popover"
      style={{ left: position.left, top: position.top }}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          close();
        }
      }}
    >
      <div className="diff-sel-header">
        {params.path}:{selection.fromLine}–{selection.toLine}
        {selection.side === "a" && <span className="muted"> (baseline)</span>}
      </div>
      <textarea
        ref={textareaRef}
        value={prompt}
        placeholder="Ask the agent about this code…  (Enter sends, ⇧Enter breaks)"
        rows={3}
        disabled={sending}
        onChange={(e) => setPrompt(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            void submit();
          }
        }}
      />
      {error && <p className="diff-sel-error">{error}</p>}
      <div className="diff-sel-actions">
        {destination && <span className="diff-sel-dest">→ {destination}</span>}
        <button onClick={close} disabled={sending}>
          Cancel
        </button>
        <button className="primary" onClick={() => void submit()} disabled={!prompt.trim() || sending}>
          {sending ? "Sending…" : "Send to agent"}
        </button>
      </div>
    </div>
  );
}

/** Popover position: near the selection's end, relative to the diff body,
 * clamped inside it. */
function computePosition(
  tabId: string,
  selection: DiffSelection,
  container: HTMLDivElement | null,
): { left: number; top: number } | null {
  const view = getDiffView(tabId);
  if (!view || !container) return null;
  const editor =
    view instanceof MergeView ? (selection.side === "a" ? view.a : view.b) : view;
  if (selection.to > editor.state.doc.length) return null;
  const coords = editor.coordsAtPos(selection.to);
  const host = container.getBoundingClientRect();
  if (!coords) return null;
  return {
    left: Math.max(4, Math.min(coords.left - host.left, host.width - 260)),
    top: Math.max(4, coords.bottom - host.top + 6),
  };
}
