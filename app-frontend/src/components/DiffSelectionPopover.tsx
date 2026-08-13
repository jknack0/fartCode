// Line-comment popover (design_handoff_v2 §5f): select lines in a diff and
// a quiet "+ comment" affordance appears; open it and an overlay card sits
// indented to the code start column. Two exits, both key-first:
//   ↵  add note   — persist a human comment on the selected range (§14)
//   ⌘↵ create task — persist the comment, then open the quick-task dialog
//                    pre-filled; the dialog owns create_task_from_comment,
//                    agent spawn, and the bidirectional comment↔task link.
// Escape closes without saving.
import { useEffect, useRef, useState, type RefObject } from "react";
import { MergeView } from "@codemirror/merge";
import { getDiffView } from "../lib/diff-views";
import { useAsyncSubmit } from "../lib/useAsyncSubmit";
import { useDiffs, type DiffParams, type DiffSelection } from "../store/diffs";
import { useLineComments } from "../store/line-comments";
import { useSidebar } from "../store/sidebar";
import { useUi } from "../store/ui";

/** Popover width (kept in sync with .lc-popover in styles/comments.css). */
const POPOVER_WIDTH = 380;

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
  const { busy: savingNote, error, setError, run } = useAsyncSubmit();
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

  if (!selection || !projectId) return null;

  const position = computePosition(tabId, selection, containerRef.current);
  if (!position) return null;

  const close = () => {
    setOpen(false);
    setPrompt("");
    setError(null);
    useDiffs.getState().setSelection(tabId, null);
  };

  // §14 "Add Note": persist a human-only comment on the selected range and
  // show the thread. The comment anchors to the diff side (a = before,
  // b/single = after).
  const sourceSide: "before" | "after" = selection.side === "a" ? "before" : "after";
  const addNote = async () => {
    const text = prompt.trim();
    if (!text || savingNote) return;
    await run(async () => {
      const firstLine = selection.text.split("\n")[0] ?? null;
      await useLineComments.getState().addNote({
        taskId,
        filePath: params.path,
        lineNumber: selection.fromLine,
        lineEnd: selection.toLine,
        sourceSide,
        lineContent: firstLine,
        content: text,
      });
      useDiffs.getState().setSelection(tabId, null);
      setPrompt("");
      // Surface the thread for this file/side so the note is visible.
      window.dispatchEvent(
        new CustomEvent("fartCode:show-comments", {
          detail: { taskId, filePath: params.path, side: sourceSide },
        }),
      );
    });
  };

  // §14 "Create Task": open the quick-task dialog pre-filled; the dialog
  // owns create_task_from_comment + agent spawn + linking.
  const createTask = () => {
    const text = prompt.trim();
    if (!text || savingNote) return;
    const pid = useSidebar.getState().selectedProjectId;
    if (!pid) return;
    // Persist the comment first so the created task links to a real row;
    // the dialog reads it back via commentId. Fire-and-forget, then open.
    void (async () => {
      try {
        const firstLine = selection.text.split("\n")[0] ?? null;
        // Through the store (not the raw command) so the comment lands in
        // byTask before the quick-task dialog's markLinked runs.
        const comment = await useLineComments.getState().addNote({
          taskId,
          filePath: params.path,
          lineNumber: selection.fromLine,
          lineEnd: selection.toLine,
          sourceSide,
          lineContent: firstLine,
          content: text,
        });
        useUi.getState().setQuickTaskTarget({
          projectId: pid,
          commentId: comment.id,
          selectedCode: selection.text,
          enclosingFunction: enclosingFunction(tabId, selection),
          prefillName: prefillNameFrom(text),
        });
        useDiffs.getState().setSelection(tabId, null);
      } catch (e) {
        setError(String(e));
      }
    })();
  };

  if (!open) {
    return (
      <button
        className="lc-fab"
        style={{ left: position.left, top: position.top }}
        onClick={() => setOpen(true)}
      >
        + comment
      </button>
    );
  }

  return (
    <div
      className="lc-popover"
      style={{ left: position.left, top: position.top }}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          close();
        }
      }}
    >
      <textarea
        ref={textareaRef}
        className="lc-input"
        value={prompt}
        placeholder="note or task…"
        rows={2}
        disabled={savingNote}
        onChange={(e) => setPrompt(e.target.value)}
        onKeyDown={(e) => {
          // stopPropagation: the global registry binds ⌘↵ (send-context)
          // even while an editor is focused — the popover owns it here.
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            e.stopPropagation();
            createTask();
          } else if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            e.stopPropagation();
            void addNote();
          }
        }}
      />
      {error && <p className="lc-error">{error}</p>}
      <div className="lc-popover-footer">
        <button
          className="lc-key-action"
          onClick={() => void addNote()}
          disabled={!prompt.trim() || savingNote}
          title="Save a note on these lines (no agent)"
        >
          <kbd>↵</kbd> add note
        </button>
        <button
          className="lc-key-action"
          onClick={createTask}
          disabled={!prompt.trim() || savingNote}
          title="Create a task for an agent to fix this"
        >
          <kbd>⌘↵</kbd> create task
        </button>
      </div>
    </div>
  );
}

/** Popover position (§5f: indented to the code start column): left edge at
 * the selection's first-line content start, top under the selection's end,
 * clamped inside the diff body. */
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
  const doc = editor.state.doc;
  const lineStart = doc.line(Math.min(selection.fromLine, doc.lines)).from;
  const startCoords = editor.coordsAtPos(lineStart);
  const endCoords = editor.coordsAtPos(selection.to);
  const host = container.getBoundingClientRect();
  if (!endCoords) return null;
  const left = startCoords ? startCoords.left - host.left : 8;
  return {
    left: Math.max(4, Math.min(left, host.width - (POPOVER_WIDTH + 8))),
    top: Math.max(4, endCoords.bottom - host.top + 6),
  };
}

/// Best-effort enclosing function/class: scan upward from the selection's
/// first line for a line that looks like a declaration. Returns null when
/// nothing plausible is found (§14: "captured best-effort").
function enclosingFunction(tabId: string, selection: DiffSelection): string | null {
  const view = getDiffView(tabId);
  if (!view) return null;
  const editor =
    view instanceof MergeView ? (selection.side === "a" ? view.a : view.b) : view;
  const doc = editor.state.doc;
  const startLine = Math.min(selection.fromLine, doc.lines);
  const declRe = /^[\t ]*(?:(?:export|public|private|protected|async|static)\s+)*(?:function|fn|def|func|class|interface|struct|enum|impl|const|let|var)\s/;
  for (let n = startLine; n >= Math.max(1, startLine - 60); n--) {
    const line = doc.line(n).text;
    if (declRe.test(line)) return line.trim();
  }
  return null;
}

/// Task name prefill: first line of the comment, trimmed to a short label.
function prefillNameFrom(content: string): string {
  const firstLine = (content.split("\n")[0] ?? "").trim();
  if (firstLine.length <= 48) return firstLine;
  return firstLine.slice(0, 48).trimEnd() + "…";
}
