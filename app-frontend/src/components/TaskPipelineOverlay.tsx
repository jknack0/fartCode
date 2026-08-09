// The task view's two pipeline overlays (ADR-0037, DESIGN.md "Overlay
// cards"): the key-first column picker and the parked-step confirm. Both
// are the app's shared overlay grammar — `.fc-overlay-card` (#17171b, .12
// hairline, radius 10) over the modal scrim, with the mono key footer
// (`esc` left, primary key right, keys in `#a4a4ab`).
//
// The copy is deliberately the board's copy: a parked step reads
// "<column> runs <provider · model · effort — trigger> on <card>.
// Dispatch?" here exactly as it does in BoardView's confirm, because it
// is the same decision about the same spend — two wordings would be two
// contracts. Both strings come from `columnConfigSummary`, the one
// formatter.
//
// Keys are captured (`useCapture`) for the same reason the delete confirm
// captures: the task view's terminal is usually focused, and xterm would
// otherwise type j/k straight into the PTY.

import { useEffect, useRef, useState } from "react";
import { columnConfigSummary, columnSublineTone } from "../lib/columnConfig";
import {
  pipelineActions,
  runFireParkedStep,
  runMoveTo,
  type TaskCardContext,
} from "../lib/taskPipeline";
import { blockerLabel, issueRefParts } from "./board/runState";
import { useTaskCard, type PipelineOverlay } from "../store/taskCard";
import type { BoardColumnDto } from "../lib/tauri";

/** A focused input owns its own keys — the same first line the board's
 * confirm handler has. Nothing in either overlay is a text field, but the
 * palette (⌘K still works underneath) is, and `j` typed there must reach
 * the search box rather than walking a hidden picker. */
const isEditableTarget = (t: EventTarget | null): boolean =>
  t instanceof HTMLElement &&
  (t.tagName === "INPUT" ||
    t.tagName === "TEXTAREA" ||
    t.tagName === "SELECT" ||
    t.isContentEditable);

export default function TaskPipelineOverlay({
  mode,
  ctx,
  defaultAgent,
}: {
  mode: PipelineOverlay;
  ctx: TaskCardContext;
  defaultAgent: string;
}) {
  const close = useTaskCard((s) => s.setOverlay);
  const summaryOf = (column: BoardColumnDto) =>
    columnConfigSummary(column, { columns: ctx.columns, defaultAgent });

  if (mode === "confirm") {
    return (
      <ConfirmParkedStep ctx={ctx} summaryOf={summaryOf} onClose={() => close(null)} />
    );
  }
  return <ColumnPicker ctx={ctx} summaryOf={summaryOf} onClose={() => close(null)} />;
}

/** Shared shell: scrim + overlay card, focused on open so the terminal
 * stops receiving keys. */
function OverlayShell({
  label,
  onClose,
  children,
}: {
  label: string;
  onClose: () => void;
  children: React.ReactNode;
}) {
  const cardRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    cardRef.current?.focus();
  }, []);
  return (
    <div className="modal-backdrop" onMouseDown={onClose}>
      <div
        ref={cardRef}
        tabIndex={-1}
        className="fc-overlay-card tv-pipeline-overlay"
        role="dialog"
        aria-label={label}
        onMouseDown={(e) => e.stopPropagation()}
      >
        {children}
      </div>
    </div>
  );
}

/** §5g/§8c confirm, task-view side: name the spend, then fire. */
function ConfirmParkedStep({
  ctx,
  summaryOf,
  onClose,
}: {
  ctx: TaskCardContext;
  summaryOf: (column: BoardColumnDto) => string;
  onClose: () => void;
}) {
  const column = pipelineActions(ctx).confirmColumn;

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (isEditableTarget(e.target)) return;
      if (e.key === "Enter") {
        e.preventDefault();
        e.stopPropagation();
        runFireParkedStep();
      } else if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onClose]);

  // The park went away underneath us (confirmed elsewhere, superseded by
  // a drag) — there is nothing left to name.
  if (!column || !ctx.issue) return null;
  const { ref, title } = issueRefParts(ctx.issue.title, ctx.issue.externalRef);

  return (
    <OverlayShell label={`Dispatch queued step in ${column.name}`} onClose={onClose}>
      <div className="fc-confirm-body">
        <div className="fc-confirm-title">
          {column.name} runs{" "}
          <span className="fc-confirm-id">{summaryOf(column)}</span> on{" "}
          {ref ? (
            <span className="fc-confirm-id">{ref}</span>
          ) : (
            <>“{title.length > 48 ? `${title.slice(0, 48)}…` : title}”</>
          )}
          . Dispatch?
        </div>
      </div>
      <div className="fc-modal-foot">
        <div className="fc-modal-foot-side">
          {/* The engine writes the move BEFORE it parks, so esc leaves the
              step parked where it is — it never puts the card back. */}
          <button type="button" onClick={onClose}>
            <span className="fc-key">esc</span> leave parked
          </button>
        </div>
        <div className="fc-modal-foot-side">
          <button type="button" onClick={() => runFireParkedStep()}>
            <span className="fc-key">↵</span> dispatch
          </button>
        </div>
      </div>
    </OverlayShell>
  );
}

/** Move to any column: j/k walks, ↵ moves, esc cancels. The move is
 * `issue_enter_column`, so a run-mode step dispatches and a queue-mode
 * step parks — the same semantics as a board drag. */
function ColumnPicker({
  ctx,
  summaryOf,
  onClose,
}: {
  ctx: TaskCardContext;
  summaryOf: (column: BoardColumnDto) => string;
  onClose: () => void;
}) {
  const columns = ctx.columns;
  // Open on the card's own column — the walk starts where the card is,
  // which is also what tells you where it is.
  const [focus, setFocus] = useState(() => {
    const i = columns.findIndex((c) => c.id === ctx.column?.id);
    return i >= 0 ? i : 0;
  });
  const focusRef = useRef(focus);
  focusRef.current = focus;

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (isEditableTarget(e.target)) return;
      const key = e.key.toLowerCase();
      const step =
        key === "j" || e.key === "ArrowDown"
          ? 1
          : key === "k" || e.key === "ArrowUp"
            ? -1
            : 0;
      if (step !== 0) {
        e.preventDefault();
        e.stopPropagation();
        setFocus((i) => Math.min(columns.length - 1, Math.max(0, i + step)));
        return;
      }
      if (e.key === "Enter") {
        e.preventDefault();
        e.stopPropagation();
        const target = columns[focusRef.current];
        if (target) runMoveTo(target);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [columns, onClose]);

  if (!ctx.issue) return null;
  const target = columns[focus] ?? null;
  const spends = target?.kind === "agent_step" && target.onEnter === "run";
  // §5g: blocked work entering a step is a confirm on the board. Here the
  // picker names it before the move rather than after the gesture.
  const activeBlockers = ctx.issue.blockers.filter((b) => !b.countsAsDone);
  const warnBlocked =
    ctx.issue.blocked && target?.kind === "agent_step" && activeBlockers.length > 0;

  return (
    <OverlayShell label="Move card to column" onClose={onClose}>
      <div className="tv-pick-list" role="listbox" aria-label="Columns">
        {columns.map((c, i) => (
          <button
            key={c.id}
            type="button"
            role="option"
            aria-selected={i === focus}
            className="tv-pick-row"
            data-focused={i === focus ? "" : undefined}
            onMouseEnter={() => setFocus(i)}
            onClick={() => runMoveTo(c)}
          >
            <span className="tv-pick-name">
              {c.name}
              {c.id === ctx.column?.id && <span className="tv-pick-here">here</span>}
            </span>
            {/* Confirm-free spend is brighter — the same tone rule the
                board headers use, so a run-mode step warns by reading
                brighter rather than by a new colour or a badge. */}
            <span className="tv-pick-sub" data-tone={columnSublineTone(c)}>
              {summaryOf(c)}
            </span>
          </button>
        ))}
      </div>
      {warnBlocked && (
        <div className="tv-pick-note">
          blocked by {activeBlockers.map((b) => blockerLabel(b.title)).join(" ")}
        </div>
      )}
      <div className="fc-modal-foot">
        <div className="fc-modal-foot-side">
          <button type="button" onClick={onClose}>
            <span className="fc-key">esc</span> cancel
          </button>
          <span aria-hidden>·</span>
          <span>
            <span className="fc-key">j</span> <span className="fc-key">k</span> walk
          </span>
        </div>
        <div className="fc-modal-foot-side">
          <button
            type="button"
            disabled={!target}
            onClick={() => target && runMoveTo(target)}
          >
            <span className="fc-key">↵</span>{" "}
            {/* The verb IS the warning: entering a run-mode step spends
                without a further confirm, so it never says "move". */}
            {spends ? "dispatch" : "move to"} {target?.name ?? ""}
          </button>
        </div>
      </div>
    </OverlayShell>
  );
}
