// Columns editor pane (#67, ADR-0037 item 9, handoff v3 §8d): the
// settings-side editing surface for a project's board columns — the board
// itself gets no editor in v1. Collapsed rows show THE column summary
// (columnConfigSummary — the same string the board headers render, one
// formatter by design); the expanded row is the full config as fc-set-*
// rows. Renders from the useColumns store and calls `reload` after every
// successful mutation so an open board picks changes up live. Occupancy
// (delete gating) derives from groupByColumn over issueList — the same
// resolution the board uses — so "N cards live here" never disagrees with
// the board.
import { useEffect, useMemo, useState } from "react";
import {
  columnCreate,
  columnDelete,
  columnReorder,
  columnUpdate,
  issueList,
  onFartcodeEvent,
  type BoardColumnDto,
  type ColumnKind,
  type IssueDto,
} from "../lib/tauri";
import {
  advanceTarget,
  columnConfigSummary,
  columnSublineTone,
  groupByColumn,
  sortColumns,
} from "../lib/columnConfig";
import { useAsyncSubmit } from "../lib/useAsyncSubmit";
import { useColumns } from "../store/columns";
import { defaultAgentName, useDependencies } from "../store/dependencies";

const KIND_LABEL: Record<ColumnKind, string> = {
  shelf: "shelf",
  agent_step: "agent step",
  human_gate: "human gate",
};

/* -- shared row shells (the ProjectSettings fc-set-* idiom) -------------- */

function Row({
  label,
  value,
  chevron,
  open,
  onClick,
  children,
}: {
  label: string;
  value: string;
  chevron?: boolean;
  open?: boolean;
  onClick?: () => void;
  children?: React.ReactNode;
}) {
  return (
    <div className={`fc-set-row-wrap${open ? " open" : ""}`}>
      <button
        type="button"
        className={`fc-set-row${onClick ? "" : " static"}`}
        onClick={onClick}
        tabIndex={onClick ? 0 : -1}
      >
        <span className="fc-set-label">{label}</span>
        <span className="fc-set-value">
          {value}
          {chevron ? " ⌄" : ""}
        </span>
      </button>
      {open && children}
    </div>
  );
}

function InlineInput({
  initial,
  placeholder,
  onSave,
  onCancel,
}: {
  initial: string;
  placeholder?: string;
  onSave: (v: string) => void;
  onCancel: () => void;
}) {
  const [v, setV] = useState(initial);
  return (
    <div className="fc-set-editor">
      <input
        autoFocus
        value={v}
        placeholder={placeholder}
        onChange={(e) => setV(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            onSave(v);
          } else if (e.key === "Escape") {
            // Keep esc local — it must not close the whole settings surface.
            e.preventDefault();
            e.stopPropagation();
            onCancel();
          }
        }}
        onBlur={() => onSave(v)}
      />
      <div className="fc-set-editor-keys">↵ save · esc cancel</div>
    </div>
  );
}

function InlineTextarea({
  initial,
  placeholder,
  rows = 3,
  keysHint = "⌘↵ save · esc cancel",
  onSave,
  onCancel,
}: {
  initial: string;
  placeholder?: string;
  rows?: number;
  keysHint?: string;
  onSave: (v: string) => void;
  onCancel: () => void;
}) {
  const [v, setV] = useState(initial);
  return (
    <div className="fc-set-editor">
      <textarea
        autoFocus
        rows={rows}
        value={v}
        placeholder={placeholder}
        onChange={(e) => setV(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            onSave(v);
          } else if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            onCancel();
          }
        }}
        onBlur={() => onSave(v)}
      />
      <div className="fc-set-editor-keys">{keysHint}</div>
    </div>
  );
}

/* -- delete gating -------------------------------------------------------- */

/** Client-side refusal reason, in spec priority order (occupied, then
 * landing, then advance-target — one at a time); null = deletable.
 * E18-07: seeded agent steps are no longer locked; a column that is
 * another column's advanceTo target is (repoint the referrer first —
 * the backend refuses it for the same spend-hazard reason). */
function deleteReason(
  column: BoardColumnDto,
  occupancy: number,
  columns: BoardColumnDto[],
): string | null {
  if (occupancy > 0) {
    return occupancy === 1
      ? "1 card lives here — move it first"
      : `${occupancy} cards live here — move them first`;
  }
  if (column.isLanding) return "landing column — move landing first";
  const referrer = columns.find((c) => c.advanceTo === column.id);
  if (referrer) return `advance target of ${referrer.name} — repoint it first`;
  return null;
}

/* -- one expanded column ------------------------------------------------- */

function ColumnDetail({
  column,
  columns,
  occupancy,
  defaultAgent,
  openField,
  setOpenField,
  patch,
  onDelete,
}: {
  column: BoardColumnDto;
  columns: BoardColumnDto[];
  occupancy: number;
  defaultAgent: string;
  openField: string | null;
  setOpenField: (f: string | null) => void;
  patch: (p: Parameters<typeof columnUpdate>[1]) => void;
  onDelete: () => void;
}) {
  const deps = useDependencies((st) => st.deps);
  const installed = deps.filter((d) => d.installed);
  const toggle = (f: string) => setOpenField(openField === f ? null : f);
  const close = () => setOpenField(null);

  const isStep = column.kind === "agent_step";
  const runsValue = [
    column.stepProvider ?? `default (${defaultAgent})`,
    column.stepModel,
    column.stepEffort,
  ]
    .filter((p): p is string => Boolean(p && p.trim()))
    .join(" · ");
  const settleTarget = advanceTarget(column, columns);
  const settleValue =
    column.onSettle === "advance"
      ? `advance → ${column.advanceTo === null ? "next column" : settleTarget?.name ?? "next column"}`
      : "hold for a human drag";
  const reason = deleteReason(column, occupancy, columns);

  return (
    <div className="fc-col-detail">
      <Row
        label="name"
        value={column.name}
        open={openField === "name"}
        onClick={() => toggle("name")}
      >
        <InlineInput
          initial={column.name}
          placeholder="column name"
          onCancel={close}
          onSave={(v) => {
            const name = v.trim();
            if (name && name !== column.name) patch({ name });
            else close();
          }}
        />
      </Row>

      <Row
        label="kind"
        value={KIND_LABEL[column.kind]}
        chevron
        open={openField === "kind"}
        onClick={() => toggle("kind")}
      >
        <div className="fc-set-editor fc-set-menu">
          {(Object.keys(KIND_LABEL) as ColumnKind[]).map((kind) => (
            <button
              key={kind}
              type="button"
              className={`fc-set-menu-item${column.kind === kind ? " current" : ""}`}
              onClick={() => patch({ kind })}
            >
              {KIND_LABEL[kind]}
            </button>
          ))}
        </div>
      </Row>

      {isStep && (
        <Row
          label="runs"
          value={runsValue}
          chevron
          open={openField === "runs"}
          onClick={() => toggle("runs")}
        >
          <div className="fc-set-editor fc-col-runs">
            <span className="fc-set-script-name">provider</span>
            <div className="fc-set-menu">
              <button
                type="button"
                className={`fc-set-menu-item${column.stepProvider === null ? " current" : ""}`}
                title="Run the app's default agent"
                onClick={() => patch({ stepProvider: null })}
              >
                default agent ({defaultAgent})
              </button>
              {installed.map((d) => (
                <button
                  key={d.providerId}
                  type="button"
                  className={`fc-set-menu-item${column.stepProvider === d.providerId ? " current" : ""}`}
                  onClick={() => patch({ stepProvider: d.providerId })}
                >
                  {d.name}
                </button>
              ))}
            </div>
            <label className="fc-set-script">
              <span className="fc-set-script-name">model</span>
              <input
                defaultValue={column.stepModel ?? ""}
                placeholder="provider default"
                onKeyDown={(e) => {
                  if (e.key === "Escape") {
                    e.preventDefault();
                    e.stopPropagation();
                    close();
                  }
                }}
                onBlur={(e) => {
                  const v = e.target.value.trim() || null;
                  if (v !== column.stepModel) patch({ stepModel: v });
                }}
              />
            </label>
            <label className="fc-set-script">
              <span className="fc-set-script-name">effort</span>
              <input
                defaultValue={column.stepEffort ?? ""}
                placeholder="provider default"
                onKeyDown={(e) => {
                  if (e.key === "Escape") {
                    e.preventDefault();
                    e.stopPropagation();
                    close();
                  }
                }}
                onBlur={(e) => {
                  const v = e.target.value.trim() || null;
                  if (v !== column.stepEffort) patch({ stepEffort: v });
                }}
              />
            </label>
            <div className="fc-set-editor-keys">saves on blur · empty clears</div>
          </div>
        </Row>
      )}

      <Row
        label="on enter"
        value={column.onEnter === "run" ? "run" : "queue — confirm first"}
        chevron
        open={openField === "onEnter"}
        onClick={() => toggle("onEnter")}
      >
        <div className="fc-set-editor fc-set-menu">
          <button
            type="button"
            className={`fc-set-menu-item${column.onEnter === "run" ? " current" : ""}`}
            onClick={() => patch({ onEnter: "run" })}
          >
            run
          </button>
          <button
            type="button"
            className={`fc-set-menu-item${column.onEnter === "queue" ? " current" : ""}`}
            onClick={() => patch({ onEnter: "queue" })}
          >
            queue — confirm first
          </button>
        </div>
      </Row>

      <Row
        label="on settle"
        value={settleValue}
        chevron
        open={openField === "onSettle"}
        onClick={() => toggle("onSettle")}
      >
        <div className="fc-set-editor fc-set-menu">
          <button
            type="button"
            className={`fc-set-menu-item${column.onSettle === "hold" ? " current" : ""}`}
            onClick={() => patch({ onSettle: "hold" })}
          >
            hold for a human drag
          </button>
          <button
            type="button"
            className={`fc-set-menu-item${column.onSettle === "advance" ? " current" : ""}`}
            onClick={() => patch({ onSettle: "advance" })}
          >
            advance
          </button>
          {column.onSettle === "advance" && (
            <>
              <span className="fc-set-script-name">advance to</span>
              <button
                type="button"
                className={`fc-set-menu-item${column.advanceTo === null ? " current" : ""}`}
                title="The next column by position"
                onClick={() => patch({ advanceTo: null })}
              >
                next column
              </button>
              {columns
                .filter((c) => c.id !== column.id)
                .map((c) => (
                  <button
                    key={c.id}
                    type="button"
                    className={`fc-set-menu-item${column.advanceTo === c.id ? " current" : ""}`}
                    onClick={() => patch({ advanceTo: c.id })}
                  >
                    {c.name}
                  </button>
                ))}
            </>
          )}
        </div>
      </Row>

      <Row
        label="counts as done"
        value={column.countsAsDone ? "on" : "off"}
        onClick={() => patch({ countsAsDone: !column.countsAsDone })}
      />

      {column.isLanding ? (
        <Row label="landing" value="landing · this column" />
      ) : (
        <Row label="landing" value="set as landing" onClick={() => patch({ isLanding: true })} />
      )}

      <Row
        label="tools"
        value={
          column.stepTools === null
            ? "—"
            : column.stepTools.length === 0
              ? "none"
              : column.stepTools.join(" · ")
        }
        open={openField === "tools"}
        onClick={() => toggle("tools")}
      >
        <InlineTextarea
          initial={(column.stepTools ?? []).join("\n")}
          placeholder={"Read\nEdit\nBash — empty allows every tool"}
          onCancel={close}
          onSave={(v) => {
            // One tool per line — entries may contain spaces ("write plan.md").
            const tools = v
              .split(/\r?\n/)
              .map((x) => x.trim())
              .filter(Boolean);
            // No-change blur must not patch: an empty textarea over a stored
            // [] would otherwise flip an allow-nothing list into unrestricted
            // (null), and every open-then-click-away would spuriously write.
            const same =
              column.stepTools === null
                ? tools.length === 0
                : tools.length === column.stepTools.length &&
                  tools.every((t, i) => t === column.stepTools![i]);
            if (same) close();
            else patch({ stepTools: tools.length > 0 ? tools : null });
          }}
        />
      </Row>

      {/* system prompt: inset-card preview, ⌘↵ (or click) opens the editor */}
      <div className="fc-col-prompt-wrap">
        <span className="fc-set-label">system prompt</span>
        {openField === "prompt" ? (
          <InlineTextarea
            initial={column.stepPrompt ?? ""}
            rows={8}
            placeholder="empty = the built-in dispatch packet"
            onCancel={close}
            onSave={(v) => {
              // No-change blur closes without a spurious write-and-reload.
              const next = v.trim() ? v : null;
              if (next !== column.stepPrompt) patch({ stepPrompt: next });
              else close();
            }}
          />
        ) : (
          <button
            type="button"
            className="fc-col-prompt"
            title="Open the system prompt editor"
            onClick={() => setOpenField("prompt")}
            onKeyDown={(e) => {
              if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                e.preventDefault();
                setOpenField("prompt");
              }
            }}
          >
            <span
              className={`fc-col-prompt-body${column.stepPrompt ? "" : " empty"}`}
            >
              {column.stepPrompt ?? "the built-in dispatch packet"}
            </span>
            <span className="fc-col-prompt-foot">⌘↵ open editor</span>
          </button>
        )}
      </div>

      <div className="fc-col-delete-row">
        {reason ? (
          <>
            <span className="fc-col-delete-disabled">delete column</span>
            <span className="fc-col-delete-reason">{reason}</span>
          </>
        ) : (
          <button type="button" className="fc-col-delete" onClick={onDelete}>
            delete column
          </button>
        )}
      </div>
    </div>
  );
}

/* -- the pane ------------------------------------------------------------- */

export function ColumnsPane({ projectId }: { projectId: string }) {
  const columnsRaw = useColumns((s) => s.byProject[projectId]);
  const loaded = useColumns((s) => Boolean(s.loaded[projectId]));
  const storeError = useColumns((s) => s.error);
  const deps = useDependencies((st) => st.deps);
  const loadDeps = useDependencies((st) => st.load);
  const [issues, setIssues] = useState<IssueDto[]>([]);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [openField, setOpenField] = useState<string | null>(null);
  const [dragId, setDragId] = useState<string | null>(null);
  // Drop target + which edge the indicator (and the insert) lands on.
  const [over, setOver] = useState<{ id: string; before: boolean } | null>(null);
  // One in-flight guard (`busy`) for every mutation: while a write+reload
  // round-trip is pending the store is stale, so a second click would compute
  // its patch (or its reorder list) from pre-mutation state. No queue — just
  // ignore. `setError` co-owns the error line with the occupancy refetches.
  const { busy, error, setError, run } = useAsyncSubmit();

  const columns = useMemo(() => sortColumns(columnsRaw ?? []), [columnsRaw]);
  const defaultAgent = defaultAgentName(deps);

  useEffect(() => {
    void useColumns.getState().load(projectId);
  }, [projectId]);

  // The runs menu names installed agents — serve the detection cache if the
  // App pane hasn't populated the store yet (ProjectSettings does the same).
  // Mount-only on purpose: a filled store must not retrigger detection.
  useEffect(() => {
    if (useDependencies.getState().deps.length === 0) void loadDeps(false);
  }, [loadDeps]);

  // Occupancy refreshes when the pane opens AND while it sits open — the
  // step engine moves cards in the background, so a mount-time snapshot
  // would let delete enable on stale data. Same event set BoardView
  // subscribes to (BoardView.tsx), same groupByColumn resolution, so the
  // delete reason never disagrees with the board.
  useEffect(() => {
    let alive = true;
    const refetch = () =>
      issueList(projectId)
        .then((list) => {
          if (alive) setIssues(list);
        })
        .catch((e) => {
          if (alive) setError(String(e));
        });
    setIssues([]);
    void refetch();
    const unlisten = onFartcodeEvent((ev) => {
      if (
        (ev.type === "issue:created" ||
          ev.type === "issue:updated" ||
          ev.type === "issue:deleted" ||
          ev.type === "step:launch" ||
          ev.type === "step:settled") &&
        ev.projectId === projectId
      ) {
        void refetch();
      }
    });
    return () => {
      alive = false;
      void unlisten.then((off) => off());
    };
  }, [projectId]);

  const occupancy = useMemo(() => groupByColumn(issues, columns), [issues, columns]);

  /** Occupancy tracks edits: displayed counts refetch with every mutation. */
  const refetchIssues = () =>
    issueList(projectId)
      .then(setIssues)
      .catch((e) => setError(String(e)));

  /** Every successful mutation reloads the store so an open board follows. */
  const mutate = (fn: () => Promise<unknown>): Promise<boolean> =>
    run(async () => {
      await fn();
      await useColumns.getState().reload(projectId);
      await refetchIssues();
    });

  const patchFor = (columnId: string) => (p: Parameters<typeof columnUpdate>[1]) => {
    if (busy) return;
    setOpenField(null);
    void mutate(() => columnUpdate(columnId, p));
  };

  const addColumn = async () => {
    await run(async () => {
      const created = await columnCreate({ projectId, name: "New column", kind: "shelf" });
      await useColumns.getState().reload(projectId);
      await refetchIssues();
      setExpandedId(created.id);
      setOpenField("name");
    });
  };

  const deleteColumn = async (columnId: string) => {
    await run(async () => {
      // Re-check occupancy on FRESH data before touching the backend — the
      // step engine may have moved a card here since the button rendered.
      // If now occupied, abort: the refreshed issues re-render the disabled
      // reason instead of surfacing the raw backend refusal.
      const fresh = await issueList(projectId);
      setIssues(fresh);
      const occupied = (groupByColumn(fresh, columns).get(columnId)?.length ?? 0) > 0;
      if (occupied) return;
      await columnDelete(columnId);
      await useColumns.getState().reload(projectId);
      // Collapse the deleted row only — the user may have expanded another
      // row mid-flight (never compare against the click-time snapshot).
      setExpandedId((cur) => (cur === columnId ? null : cur));
    });
  };

  const dropOn = (target: BoardColumnDto, e: React.DragEvent) => {
    if (busy) return;
    if (!dragId || dragId === target.id) {
      setDragId(null);
      setOver(null);
      return;
    }
    e.preventDefault();
    // The indicator already told the user which edge this drop lands on —
    // reuse the flag it was drawn from instead of recomputing.
    const before = over?.id === target.id ? over.before : false;
    const without = columns.filter((c) => c.id !== dragId);
    const dragged = columns.find((c) => c.id === dragId);
    if (!dragged) return;
    const at = without.findIndex((c) => c.id === target.id);
    const insertAt = before ? at : at + 1;
    const next = [...without.slice(0, insertAt), dragged, ...without.slice(insertAt)];
    setDragId(null);
    setOver(null);
    const ids = next.map((c) => c.id);
    // Optimistic: reposition locally, then write through and reload.
    useColumns.setState((s) => ({
      byProject: {
        ...s.byProject,
        [projectId]: next.map((c, i) => ({ ...c, position: i })),
      },
    }));
    void mutate(() => columnReorder(projectId, ids)).then((ok) => {
      if (!ok) void useColumns.getState().reload(projectId);
    });
  };

  const shown = error ?? storeError;

  if (!loaded && !shown) {
    return (
      <div className="fc-set-pane-body">
        <div className="fc-set-loading">loading…</div>
      </div>
    );
  }

  return (
    <div className="fc-set-pane-body fc-columns">
      {shown && <p className="fc-set-error">{shown}</p>}

      <div className="fc-col-list">
        {columns.map((column) => {
          const open = expandedId === column.id;
          const count = occupancy.get(column.id)?.length ?? 0;
          return (
            <div
              key={column.id}
              aria-busy={busy}
              className={`fc-col-row-wrap${open ? " open" : ""}${
                over?.id === column.id && dragId !== column.id
                  ? over.before
                    ? " drop-before"
                    : " drop-after"
                  : ""
              }`}
              onDragOver={(e) => {
                if (busy || !dragId || dragId === column.id) return;
                e.preventDefault();
                // Midpoint of the header row, not the wrapper — an expanded
                // detail would otherwise skew the edge math.
                const header =
                  (e.currentTarget as HTMLElement).querySelector(".fc-col-row") ??
                  (e.currentTarget as HTMLElement);
                const rect = header.getBoundingClientRect();
                const before = e.clientY < rect.top + rect.height / 2;
                setOver((o) =>
                  o?.id === column.id && o.before === before
                    ? o
                    : { id: column.id, before },
                );
              }}
              onDragLeave={() => setOver((o) => (o?.id === column.id ? null : o))}
              onDrop={(e) => dropOn(column, e)}
            >
              <button
                type="button"
                className="fc-col-row"
                onClick={() => {
                  setOpenField(null);
                  setExpandedId(open ? null : column.id);
                }}
              >
                <span
                  className="fc-col-handle"
                  title="Drag to reorder"
                  draggable
                  onClick={(e) => e.stopPropagation()}
                  onDragStart={(e) => {
                    if (busy) {
                      // A drag begun during a write would reorder from a
                      // stale id list (reorder-vs-create race) — suppress.
                      e.preventDefault();
                      return;
                    }
                    e.stopPropagation();
                    e.dataTransfer?.setData("text/fartCode-column", column.id);
                    if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
                    setDragId(column.id);
                  }}
                  onDragEnd={() => {
                    setDragId(null);
                    setOver(null);
                  }}
                >
                  ⋮⋮
                </span>
                <span className="fc-col-name">
                  {column.name}
                  {column.isLanding && <span className="fc-col-landing">landing</span>}
                </span>
                <span className="fc-col-summary" data-tone={columnSublineTone(column)}>
                  {columnConfigSummary(column, { columns, defaultAgent })}
                </span>
              </button>
              {open && (
                <ColumnDetail
                  column={column}
                  columns={columns}
                  occupancy={count}
                  defaultAgent={defaultAgent}
                  openField={openField}
                  setOpenField={setOpenField}
                  patch={patchFor(column.id)}
                  onDelete={() => void deleteColumn(column.id)}
                />
              )}
            </div>
          );
        })}
      </div>

      <button type="button" className="fc-col-add" onClick={() => void addColumn()}>
        + add column
      </button>

      <div className="fc-set-spacer" />
      <div className="fc-set-footer">
        <span className="fc-set-legend">seeded columns behave exactly like today until edited</span>
      </div>
    </div>
  );
}
